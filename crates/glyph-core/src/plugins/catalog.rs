use super::*;

// ------------------------------------------------------------------ catalog

/// One installable plugin from the catalog.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Listing {
    /// `owner/name` on GitHub.
    pub repo: String,
    /// Folder of the manifest inside the repo; empty = root.
    #[serde(default)]
    pub path: String,
    /// Commit (checked) or `HEAD` (community).
    pub rev: String,
    /// Listed in `glyphmusic/plugins` (checked by CI and a maintainer).
    #[serde(default)]
    pub checked: bool,
    /// GitHub stars of the repo; `None` for plugins in a folder of a shared
    /// repo (the repo's stars are not theirs).
    #[serde(default)]
    pub stars: Option<u32>,
    /// Last push to the repo (ISO 8601), for "recently updated".
    #[serde(default)]
    pub updated: Option<String>,
    pub manifest: Manifest,
}

impl Listing {
    pub fn repo_url(&self) -> String {
        if self.path.is_empty() {
            format!("https://github.com/{}", self.repo)
        } else {
            format!("https://github.com/{}/tree/{}/{}", self.repo, self.rev, self.path)
        }
    }

    /// May be installed: valid for this Glyph, and code only when checked.
    pub fn installable(&self) -> Result<(), Invalid> {
        self.manifest.validate(app_version())?;
        if !self.checked && !self.manifest.data_only() {
            return Err(Invalid::Bad("only checked plugins may run code".into()));
        }
        Ok(())
    }
}

/// `index.json` of `glyphmusic/plugins`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Index {
    #[serde(default)]
    pub plugins: Vec<IndexEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexEntry {
    pub repo: String,
    #[serde(default)]
    pub path: String,
    pub rev: String,
    /// Filled in daily by the index CI (repo stars and last push).
    #[serde(default)]
    pub stars: Option<u32>,
    #[serde(default)]
    pub updated: Option<String>,
    pub manifest: Manifest,
}

/// A repo found by topic (the search API's fields we use).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TopicRepo {
    pub full_name: String,
    #[serde(default)]
    pub stargazers_count: u32,
    #[serde(default)]
    pub archived: bool,
    #[serde(default)]
    pub pushed_at: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Catalog {
    /// Unix seconds of the last refresh.
    pub fetched_at: u64,
    pub checked: Vec<Listing>,
    pub community: Vec<Listing>,
    /// ETag of the last topic search: an unchanged answer (304) does not
    /// count against GitHub's rate limit.
    #[serde(default)]
    pub search_etag: Option<String>,
    #[serde(default)]
    pub topic_repos: Vec<TopicRepo>,
    /// The last refresh could not reach one of the sources (shown, the
    /// cached part stays).
    #[serde(default)]
    pub partial: bool,
}

impl Catalog {
    /// Every listing, checked first.
    pub fn all(&self) -> impl Iterator<Item = &Listing> {
        self.checked.iter().chain(self.community.iter())
    }

    pub fn find(&self, id: &str) -> Option<&Listing> {
        self.all().find(|l| l.manifest.id == id)
    }
}

pub fn plugins_dir() -> PathBuf {
    paths::data_dir().join("plugins")
}

fn catalog_path() -> PathBuf {
    plugins_dir().join("catalog.json")
}

/// The cached catalog, however old.
pub fn cached_catalog() -> Option<Catalog> {
    paths::read_json(&catalog_path())
}

/// The catalog: cached while younger than an hour, unless `force`.
pub async fn catalog(http: &reqwest::Client, force: bool) -> Result<Catalog> {
    let cached = cached_catalog();
    if let Some(c) = &cached {
        if !force && !c.partial && paths::now_unix().saturating_sub(c.fetched_at) < CATALOG_TTL_S {
            return Ok(c.clone());
        }
    }
    let prev = cached.unwrap_or_default();
    let (index, search) = futures::join!(fetch_index(http), search_topic(http, prev.search_etag.as_deref()));

    let mut partial = false;
    let mut checked = match index {
        Ok(i) => listings_from_index(i),
        Err(e) => {
            tracing::warn!("plugin index: {e:#}");
            partial = true;
            prev.checked.clone()
        }
    };
    let (topic_repos, etag, refetch) = match search {
        Ok(Search::Fresh(repos, etag)) => (repos, etag, true),
        Ok(Search::Unchanged) => (prev.topic_repos.clone(), prev.search_etag.clone(), false),
        Err(e) => {
            tracing::warn!("plugin topic search: {e:#}");
            partial = true;
            (prev.topic_repos.clone(), prev.search_etag.clone(), false)
        }
    };
    // The topic search is fresher than the index's daily stars.
    for l in checked.iter_mut().filter(|l| l.path.is_empty()) {
        if let Some(r) = topic_repos.iter().find(|r| r.full_name.eq_ignore_ascii_case(&l.repo)) {
            l.stars = Some(r.stargazers_count);
            if !r.pushed_at.is_empty() {
                l.updated = Some(r.pushed_at.clone());
            }
        }
    }
    let community = if refetch || prev.community.is_empty() || force {
        community_listings(http, &topic_repos, &checked).await
    } else {
        prev.community.into_iter().filter(|l| !checked.iter().any(|c| c.manifest.id == l.manifest.id || c.repo.eq_ignore_ascii_case(&l.repo))).collect()
    };
    if checked.is_empty() && community.is_empty() && partial {
        bail!("plugin catalog unreachable");
    }
    let c = Catalog { fetched_at: paths::now_unix(), checked, community, search_etag: etag, topic_repos, partial };
    if let Err(e) = paths::write_json(&catalog_path(), &c) {
        tracing::warn!("plugin catalog cache: {e:#}");
    }
    Ok(c)
}

async fn fetch_index(http: &reqwest::Client) -> Result<Index> {
    let r = http.get(INDEX_URL).send().await?.error_for_status()?;
    let bytes = r.bytes().await?;
    serde_json::from_slice(&bytes).context("index.json")
}

/// Checked listings, dropping malformed entries and duplicate ids.
pub(super) fn listings_from_index(index: Index) -> Vec<Listing> {
    let mut out: Vec<Listing> = Vec::new();
    for e in index.plugins {
        if !valid_repo(&e.repo) || !valid_path(&e.path) || !valid_id(&e.manifest.id) || out.iter().any(|l| l.manifest.id == e.manifest.id) {
            continue;
        }
        let own_repo = e.path.is_empty();
        out.push(Listing {
            repo: e.repo,
            path: e.path,
            rev: e.rev,
            checked: true,
            stars: e.stars.filter(|_| own_repo),
            updated: e.updated,
            manifest: e.manifest,
        });
    }
    out.sort_by(|a, b| a.manifest.name.to_lowercase().cmp(&b.manifest.name.to_lowercase()));
    out
}

enum Search {
    Fresh(Vec<TopicRepo>, Option<String>),
    Unchanged,
}

async fn search_topic(http: &reqwest::Client, etag: Option<&str>) -> Result<Search> {
    #[derive(Deserialize)]
    struct Page {
        items: Vec<TopicRepo>,
    }
    let mut req = http.get(SEARCH_URL).header("accept", "application/vnd.github+json").header("x-github-api-version", "2022-11-28");
    if let Some(e) = etag {
        req = req.header("if-none-match", e);
    }
    let r = req.send().await?;
    if r.status() == reqwest::StatusCode::NOT_MODIFIED {
        return Ok(Search::Unchanged);
    }
    if r.status() == reqwest::StatusCode::FORBIDDEN || r.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
        bail!("GitHub rate limit reached");
    }
    let r = r.error_for_status()?;
    let etag = r.headers().get("etag").and_then(|v| v.to_str().ok()).map(str::to_string);
    let page: Page = serde_json::from_slice(&r.bytes().await?).context("topic search")?;
    let repos = page.items.into_iter().filter(|r| !r.archived && valid_repo(&r.full_name)).collect();
    Ok(Search::Fresh(repos, etag))
}

/// Fetches the manifest of each topic repo not already checked; repos
/// without a valid manifest are left out.
async fn community_listings(http: &reqwest::Client, repos: &[TopicRepo], checked: &[Listing]) -> Vec<Listing> {
    let todo: Vec<TopicRepo> = repos
        .iter()
        .filter(|r| !checked.iter().any(|c| c.repo.eq_ignore_ascii_case(&r.full_name)))
        .take(COMMUNITY_MAX)
        .cloned()
        .collect();
    let mut found: Vec<Listing> = futures::stream::iter(todo)
        .map(|r| async move {
            let m = fetch_manifest(http, &r.full_name, "HEAD", "").await.ok()?;
            let updated = (!r.pushed_at.is_empty()).then(|| r.pushed_at.clone());
            Some(Listing { repo: r.full_name, path: String::new(), rev: "HEAD".into(), checked: false, stars: Some(r.stargazers_count), updated, manifest: m })
        })
        .buffer_unordered(8)
        .filter_map(|l| async move { l })
        .collect()
        .await;
    // Ids are unique: checked ones win, then the most starred repo.
    found.sort_by(|a, b| b.stars.unwrap_or(0).cmp(&a.stars.unwrap_or(0)).then_with(|| a.repo.cmp(&b.repo)));
    let mut out: Vec<Listing> = Vec::new();
    for l in found {
        if valid_id(&l.manifest.id) && !checked.iter().chain(out.iter()).any(|c| c.manifest.id == l.manifest.id) {
            out.push(l);
        }
    }
    out
}

/// `glyph-plugin.json` of `repo` at `rev` (raw.githubusercontent.com has no
/// API rate limit).
pub async fn fetch_manifest(http: &reqwest::Client, repo: &str, rev: &str, path: &str) -> Result<Manifest> {
    if !valid_repo(repo) || !valid_path(path) || !rev.bytes().all(|b| b.is_ascii_alphanumeric() || b"-_./".contains(&b)) {
        bail!("invalid plugin location");
    }
    let file = if path.is_empty() { MANIFEST_FILE.to_string() } else { format!("{path}/{MANIFEST_FILE}") };
    let url = format!("https://raw.githubusercontent.com/{repo}/{rev}/{file}");
    let r = http.get(url).send().await?.error_for_status()?;
    let bytes = r.bytes().await?;
    if bytes.len() > MAX_MANIFEST {
        bail!("manifest too large");
    }
    serde_json::from_slice(&bytes).context(MANIFEST_FILE)
}

// ------------------------------------------------------------------ search

/// How the catalog is ordered (Settings -> Plugins, glyph.my/plugins).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Sort {
    /// Most GitHub stars first.
    #[default]
    Stars,
    /// Most recently pushed first.
    Updated,
    Name,
}

/// Fuzzy match of `query` in `text`: `None` unless every query character
/// appears in order (case-insensitive). Higher is better: characters at word
/// starts and runs of consecutive characters count more, gaps less, an exact
/// substring (above all at the start) most.
pub fn fuzzy_score(query: &str, text: &str) -> Option<i32> {
    let q: Vec<char> = query.to_lowercase().chars().filter(|c| !c.is_whitespace()).collect();
    if q.is_empty() {
        return Some(0);
    }
    let lower = text.to_lowercase();
    let t: Vec<char> = lower.chars().collect();
    let (mut score, mut qi, mut prev) = (0i32, 0usize, None::<usize>);
    for (i, c) in t.iter().enumerate() {
        if qi == q.len() {
            break;
        }
        if *c != q[qi] {
            continue;
        }
        score += 1;
        if i == 0 || !t[i - 1].is_alphanumeric() {
            score += 8;
        }
        if let Some(p) = prev {
            score += if p + 1 == i { 5 } else { -((i - p - 1) as i32).min(3) };
        }
        prev = Some(i);
        qi += 1;
    }
    if qi < q.len() {
        return None;
    }
    let whole = query.trim().to_lowercase();
    if lower.starts_with(&whole) {
        score += 30;
    } else if lower.contains(&whole) {
        score += 20;
    }
    Some(score)
}

/// Best match of `query` in a listing: name first, then id, author and
/// description.
pub fn search_score(l: &Listing, query: &str) -> Option<i32> {
    let m = &l.manifest;
    [
        fuzzy_score(query, &m.name).map(|s| s + 15),
        fuzzy_score(query, &m.id).map(|s| s + 10),
        fuzzy_score(query, &m.author),
        fuzzy_score(query, &m.description).map(|s| s - 5),
    ]
    .into_iter()
    .flatten()
    .max()
}

/// `items` matching `query` (all for an empty one), best matches first while
/// searching, otherwise (and among equal matches) in `sort` order.
pub fn filter_sort<'a>(items: &'a [Listing], query: &str, sort: Sort) -> Vec<&'a Listing> {
    let searching = !query.trim().is_empty();
    let mut hits: Vec<(i32, &Listing)> = items
        .iter()
        .filter_map(|l| if searching { search_score(l, query).map(|s| (s, l)) } else { Some((0, l)) })
        .collect();
    hits.sort_by(|(sa, a), (sb, b)| {
        sb.cmp(sa)
            .then_with(|| match sort {
                Sort::Stars => b.stars.unwrap_or(0).cmp(&a.stars.unwrap_or(0)),
                // ISO 8601 strings order like the times they stand for.
                Sort::Updated => b.updated.cmp(&a.updated),
                Sort::Name => std::cmp::Ordering::Equal,
            })
            .then_with(|| a.manifest.name.to_lowercase().cmp(&b.manifest.name.to_lowercase()))
    });
    hits.into_iter().map(|(_, l)| l).collect()
}
