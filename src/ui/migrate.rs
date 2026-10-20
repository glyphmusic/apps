//! Settings -> Transfer music: moves a whole library from another service
//! (liked songs, playlists, saved albums, followed artists) in one go.
//! Nobody logs in or registers anything: Spotify / YouTube Music hand over
//! what was copied in their app (clipboard) or a Google Takeout file, Deezer
//! and SoundCloud a public profile, Last.fm a user name. Then: choose what
//! comes over, run in the background, result. Single playlists come over by
//! link (New playlist dialog / search bar) or as a file.
//! Logic: `glyph_core::import::accounts`; this file holds the UI state.

use crate::app::{Glyph, SettingsPage, View};
use crate::guard::guard;
use crate::i18n::{t, tf};
use crate::theme;
use crate::ui::input::TextInput;
use crate::ui::settings::{checkbox, input_box, row, rows, section_label, white};
use crate::ui::widgets::*;
use glyph_core::flume_like;
use glyph_core::import::accounts::{
    Collection, Connect, Kind, LibrarySource, Login, MigrationEvent, MigrationState, Migrated, Overview, Service, Totals,
};
use gpui::{div, prelude::*, px, AnyElement, Context, Div, Entity, FontWeight, SharedString};
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

#[derive(Default)]
pub enum Step {
    #[default]
    Pick,
    Connect(Service),
    Loading(Service),
    Choose(Choice),
    Running(Run),
    Done(Finished),
}

pub struct Choice {
    pub service: Service,
    pub source: Arc<dyn LibrarySource>,
    pub overview: Overview,
    pub picked: HashSet<String>,
    /// What was handed over (kept so a stopped run can continue).
    pub login: Login,
    /// Collections already moved by an earlier, stopped run.
    pub done_before: Vec<String>,
}

pub struct Run {
    pub service: Service,
    pub index: usize,
    pub lists: usize,
    pub current: String,
    pub done: usize,
    pub total: usize,
    pub totals: Totals,
    pub failed: Vec<(String, String)>,
    pub notes: Vec<String>,
    pub cancel: Arc<AtomicBool>,
    pub state: MigrationState,
}

pub struct Finished {
    pub service: Service,
    pub totals: Totals,
    pub failed: Vec<(String, String)>,
    pub notes: Vec<String>,
    pub cancelled: bool,
}

#[derive(Default)]
pub struct MigrateUi {
    pub step: Step,
    pub error: Option<String>,
    profile: Option<Entity<TextInput>>,
    /// Copied text with links (Spotify / YouTube Music step).
    pasted: Option<String>,
    /// (songs, lists) found in `pasted`.
    pasted_counts: (usize, usize),
    /// Picked files (Google Takeout CSVs, exports).
    files: Vec<PathBuf>,
    /// Unfinished run from an earlier start (None: not looked at yet).
    resume: Option<Option<MigrationState>>,
}

impl MigrateUi {
    /// A move is running (sidebar / library show its progress).
    pub fn running(&self) -> Option<&Run> {
        match &self.step {
            Step::Running(r) => Some(r),
            _ => None,
        }
    }
}

fn value(e: &Option<Entity<TextInput>>, cx: &Context<Glyph>) -> String {
    e.as_ref().map(|i| i.read(cx).value.trim().to_string()).unwrap_or_default()
}

/// Display name of a collection.
pub fn collection_name(c: &Collection) -> String {
    match c.kind {
        Kind::Liked => t("migrate.liked").to_string(),
        Kind::Albums => t("migrate.albums").to_string(),
        Kind::Artists => t("migrate.artists").to_string(),
        Kind::Playlist if c.name.trim().is_empty() => c.hint.clone(),
        Kind::Playlist => c.name.clone(),
    }
}

/// The service a pasted link list most likely comes from.
fn guess_service(text: &str) -> Service {
    let l = text.to_lowercase();
    if l.contains("youtube.com") || l.contains("youtu.be") {
        Service::YouTube
    } else {
        Service::Spotify
    }
}

impl Glyph {
    /// Opens Settings -> Transfer music.
    pub fn open_migrate(&mut self, cx: &mut Context<Self>) {
        self.open_settings(Some(SettingsPage::Migrate), cx);
    }

    fn ensure_migrate_inputs(&mut self, cx: &mut Context<Self>) {
        if self.migrate.profile.is_none() {
            self.migrate.profile = Some(cx.new(|cx| TextInput::new("migrate.profile.placeholder", cx)));
        }
    }

    /// Step 2 for `service`.
    pub fn migrate_pick(&mut self, service: Service, profile: Option<String>, cx: &mut Context<Self>) {
        self.ensure_migrate_inputs(cx);
        if let Some(e) = self.migrate.profile.clone() {
            e.update(cx, |i, cx| i.set_value(profile.unwrap_or_default(), cx));
        }
        let m = &mut self.migrate;
        m.pasted = None;
        m.pasted_counts = (0, 0);
        m.files.clear();
        m.error = None;
        m.step = Step::Connect(service);
        cx.notify();
    }

    /// A text with several song / playlist links was pasted into the search
    /// bar: open Transfer music with it.
    pub fn migrate_open_pasted(&mut self, text: String, cx: &mut Context<Self>) {
        let service = guess_service(&text);
        self.migrate_pick(service, None, cx);
        self.migrate_set_pasted(text);
        self.open_migrate(cx);
    }

    fn migrate_set_pasted(&mut self, text: String) {
        let counts = glyph_core::Backend::pasted_links(&text);
        let m = &mut self.migrate;
        if counts == (0, 0) {
            m.error = Some(t("migrate.error.clipboard").to_string());
            return;
        }
        m.error = None;
        m.pasted_counts = counts;
        m.pasted = Some(text);
    }

    /// Takes the links copied in the other app from the clipboard.
    pub fn migrate_paste_clipboard(&mut self, cx: &mut Context<Self>) {
        let text = cx.read_from_clipboard().and_then(|c| c.text()).unwrap_or_default();
        self.migrate_set_pasted(text);
        cx.notify();
    }

    /// Picks files (Google Takeout CSVs, playlist exports).
    pub fn migrate_pick_files(&mut self, cx: &mut Context<Self>) {
        let rx = cx.prompt_for_paths(gpui::PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some(t("migrate.files.pick").into()),
        });
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(paths))) = rx.await else { return };
            this.update(cx, |s, cx| {
                for p in paths {
                    if !s.migrate.files.contains(&p) {
                        s.migrate.files.push(p);
                    }
                }
                s.migrate.error = None;
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub fn migrate_back(&mut self, cx: &mut Context<Self>) {
        self.migrate.error = None;
        self.migrate.resume = None;
        self.migrate.step = Step::Pick;
        cx.notify();
    }

    /// Reads what was handed over, then shows what can come over.
    pub fn migrate_connect(&mut self, cx: &mut Context<Self>) {
        let Step::Connect(service) = self.migrate.step else { return };
        self.migrate.error = None;
        let login = match service.connect() {
            Connect::Paste { .. } => {
                if self.migrate.pasted.is_none() && self.migrate.files.is_empty() {
                    self.migrate.error = Some(t("migrate.error.nothing").to_string());
                    cx.notify();
                    return;
                }
                Login::Pasted { text: self.migrate.pasted.clone().unwrap_or_default(), files: self.migrate.files.clone() }
            }
            Connect::Profile | Connect::UserName => {
                let profile = value(&self.migrate.profile, cx);
                if profile.is_empty() {
                    self.migrate.error = Some(t("migrate.error.profile").to_string());
                    cx.notify();
                    return;
                }
                Login::Public(profile)
            }
        };
        self.migrate_load(service, login, cx);
    }

    /// Continues a stopped run with what was handed over then.
    pub fn migrate_resume(&mut self, cx: &mut Context<Self>) {
        let Some(st) = self.migrate.resume.clone().flatten() else { return };
        let login = match (&st.pasted, &st.profile) {
            (Some(text), _) => Login::Pasted { text: text.clone(), files: st.files.clone() },
            (None, Some(p)) => Login::Public(p.clone()),
            (None, None) if !st.files.is_empty() => Login::Pasted { text: String::new(), files: st.files.clone() },
            _ => {
                self.migrate_pick(st.service, None, cx);
                return;
            }
        };
        self.migrate_load(st.service, login, cx);
    }

    fn migrate_load(&mut self, service: Service, login: Login, cx: &mut Context<Self>) {
        let Some(b) = self.backend.clone() else {
            self.migrate.error = Some(t("status.library_offline").to_string());
            cx.notify();
            return;
        };
        let source = match b.library_source(service, login.clone(), self.config.keys.clone()) {
            Ok(s) => s,
            Err(e) => {
                self.migrate.error = Some(format!("{e:#}"));
                self.migrate.step = Step::Connect(service);
                cx.notify();
                return;
            }
        };
        self.migrate.step = Step::Loading(service);
        cx.notify();
        cx.spawn(async move |this, cx| {
            let r = crate::guard::guard_async(b.library_overview(source.clone())).await;
            this.update(cx, |s, cx| {
                if !matches!(s.migrate.step, Step::Loading(sv) if sv == service) {
                    return;
                }
                match r {
                    Ok(overview) => {
                        // A stopped run of the same service: pick what is left.
                        let resume = MigrationState::load().filter(|st| st.service == service);
                        let (picked, done_before) = match &resume {
                            Some(st) => (st.remaining().into_iter().map(|c| c.id).collect(), st.done.clone()),
                            None => (overview.collections.iter().filter(|c| c.default_on).map(|c| c.id.clone()).collect(), Vec::new()),
                        };
                        s.migrate.step = Step::Choose(Choice { service, source, overview, picked, login, done_before });
                    }
                    Err(e) => {
                        s.migrate.error = Some(e);
                        s.migrate.step = Step::Connect(service);
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub fn migrate_toggle(&mut self, id: &str, cx: &mut Context<Self>) {
        if let Step::Choose(c) = &mut self.migrate.step {
            if !c.picked.remove(id) {
                c.picked.insert(id.to_string());
            }
            cx.notify();
        }
    }

    /// Ticks (or unticks) every playlist.
    pub fn migrate_all_playlists(&mut self, on: bool, cx: &mut Context<Self>) {
        if let Step::Choose(c) = &mut self.migrate.step {
            for col in c.overview.collections.iter().filter(|c| c.kind == Kind::Playlist) {
                if on {
                    c.picked.insert(col.id.clone());
                } else {
                    c.picked.remove(&col.id);
                }
            }
            cx.notify();
        }
    }

    /// Starts the move of everything ticked.
    pub fn migrate_start(&mut self, cx: &mut Context<Self>) {
        let Some(b) = self.backend.clone() else { return };
        if !self.library_ok {
            self.migrate.error = Some(t("status.library_offline").to_string());
            cx.notify();
            return;
        }
        if !matches!(self.migrate.step, Step::Choose(_)) {
            return;
        }
        let Step::Choose(choice) = std::mem::take(&mut self.migrate.step) else { return };
        // Liked songs first, then playlists, albums, artists (overview order).
        let mut selected: Vec<Collection> = choice.overview.collections.iter().filter(|c| choice.picked.contains(&c.id)).cloned().collect();
        selected.sort_by_key(|c| match c.kind {
            Kind::Liked => 0,
            Kind::Playlist => 1,
            Kind::Albums => 2,
            Kind::Artists => 3,
        });
        if selected.is_empty() {
            self.migrate.step = Step::Choose(choice);
            return;
        }
        let (profile, pasted, files) = match &choice.login {
            Login::Public(p) => (Some(p.clone()), None, Vec::new()),
            Login::Pasted { text, files } => (None, Some(text.clone()).filter(|t| !t.is_empty()), files.clone()),
        };
        let state = MigrationState {
            service: choice.service,
            profile,
            pasted,
            files,
            account: choice.overview.account.clone(),
            selected: selected.clone(),
            done: Vec::new(),
            started: glyph_core::paths::now_unix(),
        };
        if let Err(e) = state.save() {
            tracing::warn!("migration state: {e:#}");
        }
        let cancel = Arc::new(AtomicBool::new(false));
        self.migrate.resume = None;
        self.migrate.step = Step::Running(Run {
            service: choice.service,
            index: 0,
            lists: selected.len(),
            current: String::new(),
            done: 0,
            total: 0,
            totals: Totals::default(),
            failed: Vec::new(),
            notes: choice.overview.notes.clone(),
            cancel: cancel.clone(),
            state,
        });
        self.toast_sub(t("migrate.started"), choice.service.name());
        cx.notify();
        let (tx, mut rx) = flume_like::channel::<MigrationEvent>();
        cx.spawn(async move |this, cx| {
            while let Some(ev) = rx.recv().await {
                if this.update(cx, |s, cx| s.on_migrate_event(ev, cx)).is_err() {
                    break;
                }
            }
        })
        .detach();
        let source = choice.source;
        cx.spawn(async move |this, cx| {
            if let Err(e) = crate::guard::guard_async(b.migrate(source, selected, tx, cancel)).await {
                this.update(cx, |s, cx| {
                    s.migrate.error = Some(e);
                    cx.notify();
                })
                .ok();
            }
        })
        .detach();
    }

    fn on_migrate_event(&mut self, ev: MigrationEvent, cx: &mut Context<Self>) {
        let Step::Running(run) = &mut self.migrate.step else { return };
        match ev {
            MigrationEvent::Collection { index, total, collection } => {
                run.index = index;
                run.lists = total;
                run.current = collection_name(&collection);
                run.done = 0;
                run.total = 0;
            }
            MigrationEvent::Progress { done, total } => {
                run.done = done;
                run.total = total;
            }
            MigrationEvent::Failed { collection, error } => {
                run.failed.push((collection_name(&collection), error));
                run.state.mark_done(&collection.id);
                let _ = run.state.save();
            }
            MigrationEvent::Done { collection, result } => {
                run.totals.add(&collection, &result);
                run.state.mark_done(&collection.id);
                let _ = run.state.save();
                self.apply_migrated(&collection, result, cx);
            }
            MigrationEvent::Finished { cancelled } => {
                let Step::Running(run) = std::mem::take(&mut self.migrate.step) else { return };
                if !cancelled {
                    MigrationState::clear();
                }
                let pct = run.totals.percent();
                if cancelled {
                    self.toast_info(t("migrate.stopped"));
                } else {
                    self.toast_sub(t("migrate.finished"), format!("{} · {pct}%", run.service.name()));
                }
                self.migrate.step = Step::Done(Finished { service: run.service, totals: run.totals, failed: run.failed, notes: run.notes, cancelled });
            }
        }
        cx.notify();
    }

    /// Saves one moved collection into the library.
    fn apply_migrated(&mut self, c: &Collection, result: Migrated, cx: &mut Context<Self>) {
        let ok = guard(|| match result {
            Migrated::Tracks(res) => match c.kind {
                Kind::Liked => {
                    self.library.like_all(res.ordered);
                }
                _ => {
                    let name = if c.name.trim().is_empty() { res.name.clone() } else { c.name.clone() };
                    self.library.save_import(&name, c.origin.as_deref(), res.ordered);
                }
            },
            Migrated::Albums(albums) => {
                self.library.save_albums(albums.into_iter().map(|a| (a.title, a.artist, a.tracks)));
            }
            Migrated::Artists(names) => {
                self.library.follow_all(names);
            }
        });
        if ok.is_some() {
            self.save_library(cx);
        }
    }

    /// Stops after the song being matched; what is saved stays.
    pub fn migrate_cancel(&mut self, cx: &mut Context<Self>) {
        if let Step::Running(r) = &self.migrate.step {
            r.cancel.store(true, Ordering::Relaxed);
        }
        cx.notify();
    }

    /// Forgets a stopped move.
    pub fn migrate_discard_resume(&mut self, cx: &mut Context<Self>) {
        MigrationState::clear();
        self.migrate.resume = Some(None);
        cx.notify();
    }
}

// ------------------------------------------------------------------ UI

fn connect_desc(s: Service) -> SharedString {
    t(&format!("migrate.service.{}.desc", s.key()))
}

fn note(text: impl Into<SharedString>) -> Div {
    div().pt(px(14.0)).text_size(px(12.0)).text_color(white(0.4)).child(text.into())
}

fn error_line(e: &str) -> Div {
    div()
        .mt(px(12.0))
        .flex()
        .items_start()
        .gap(px(8.0))
        .px(px(12.0))
        .py(px(10.0))
        .rounded(px(8.0))
        .bg(white(0.05))
        .text_size(px(13.0))
        .text_color(white(0.85))
        .child(div().flex_none().pt(px(1.0)).child(icon("info", 14.0, white(0.6))))
        .child(div().min_w_0().child(e.to_string()))
}

/// Mono count next to a collection (`1.204`).
fn count_chip(n: Option<usize>) -> Div {
    mono()
        .px(px(6.0))
        .rounded(px(4.0))
        .bg(white(0.08))
        .text_size(px(11.0))
        .text_color(white(0.6))
        .child(n.map(|n| n.to_string()).unwrap_or_else(|| "?".into()))
}

/// Which step to draw (copied out, so the step builders may borrow `app`).
enum Show {
    Pick,
    Connect(Service),
    Loading(Service),
    Choose,
    Running,
    Done,
}

pub fn page(app: &mut Glyph, cx: &mut Context<Glyph>) -> Div {
    app.ensure_migrate_inputs(cx);
    let show = match &app.migrate.step {
        Step::Pick => Show::Pick,
        Step::Connect(s) => Show::Connect(*s),
        Step::Loading(s) => Show::Loading(*s),
        Step::Choose(_) => Show::Choose,
        Step::Running(_) => Show::Running,
        Step::Done(_) => Show::Done,
    };
    let body = match show {
        Show::Pick => pick_page(app, cx),
        Show::Connect(s) => connect_page(app, s, cx),
        Show::Loading(s) => div()
            .flex()
            .flex_col()
            .gap(px(12.0))
            .child(section_label(s.name()))
            .child(spinner_line(app.spinner(), t("migrate.loading")))
            .child(skeleton_rows("migrate-sk", 5)),
        Show::Choose => choose_page(app, cx),
        Show::Running => running_page(app, cx),
        Show::Done => done_page(app, cx),
    };
    let err = app.migrate.error.clone();
    div().flex().flex_col().child(body).children(err.map(|e| error_line(&e)))
}

fn pick_page(app: &mut Glyph, cx: &mut Context<Glyph>) -> Div {
    if app.migrate.resume.is_none() {
        app.migrate.resume = Some(MigrationState::load());
    }
    let resume = app.migrate.resume.clone().flatten().map(|st| {
        let (left, all) = (st.remaining().len(), st.selected.len());
        let service = st.service;
        rows(vec![row(
            tf("migrate.resume", &[("service", &service.name())]),
            Some(tf("migrate.resume.desc", &[("done", &(all - left)), ("all", &all), ("account", &st.account)])),
            div()
                .flex()
                .gap(px(8.0))
                .child(text_button("mg-resume-drop", t("migrate.resume.discard")).on_click(cx.listener(|this, _, _, cx| this.migrate_discard_resume(cx))))
                .child(primary_button("mg-resume", t("migrate.resume.button")).on_click(cx.listener(|this, _, _, cx| this.migrate_resume(cx)))),
        )])
    });
    let services = rows(
        Service::ALL
            .into_iter()
            .map(|s| {
                row(
                    s.name(),
                    Some(connect_desc(s)),
                    button(SharedString::from(format!("mg-{}", s.key())), t("migrate.connect"))
                        .child(icon("arrow-right", 14.0, white(0.6)))
                        .on_click(cx.listener(move |this, _, _, cx| this.migrate_pick(s, None, cx))),
                )
            })
            .collect(),
    );
    let single = rows(vec![
        row(
            t("migrate.link"),
            Some(t("migrate.link.desc")),
            button("mg-link", t("migrate.link.button"))
                .child(icon("link-2", 14.0, white(0.6)))
                .on_click(cx.listener(|this, _, w, cx| {
                    this.close_settings(w, cx);
                    this.open_name_dialog(Vec::new(), None, w, cx);
                })),
        ),
        row(
            t("migrate.file"),
            Some(t("migrate.file.desc")),
            button("mg-file", t("migrate.file.button"))
                .child(icon("folder-open", 14.0, white(0.6)))
                .on_click(cx.listener(|this, _, w, cx| {
                    this.close_settings(w, cx);
                    this.import_playlist_file(cx);
                })),
        ),
    ]);
    div()
        .flex()
        .flex_col()
        .when_some(resume, |d, r| d.child(section_label(t("migrate.resume.section"))).child(r))
        .child(section_label(t("migrate.from")))
        .child(services)
        .child(section_label(t("migrate.single")))
        .child(single)
        .child(note(t("migrate.note")))
}

/// Numbered how-to lines `migrate.<service>.step1..n`.
fn steps(s: Service, n: usize) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .gap(px(6.0))
        .py(px(12.0))
        .text_size(px(13.0))
        .text_color(white(0.75))
        .children((1..=n).map(|i| {
            div()
                .flex()
                .gap(px(10.0))
                .child(mono().flex_none().text_color(white(0.45)).child(format!("{i}.")))
                .child(div().min_w_0().child(t(&format!("migrate.{}.step{i}", s.key()))))
        }))
        .into_any_element()
}

fn connect_page(app: &mut Glyph, s: Service, cx: &mut Context<Glyph>) -> Div {
    let m = &app.migrate;
    let back = text_button("mg-back", t("migrate.back")).on_click(cx.listener(|this, _, _, cx| this.migrate_back(cx)));
    let mut list = Vec::new();
    let ready = match s.connect() {
        Connect::Paste { takeout } => {
            list.push(steps(s, 3));
            let (songs, lists) = m.pasted_counts;
            let found = m.pasted.is_some().then(|| tf("migrate.pasted", &[("songs", &songs), ("lists", &lists)]));
            list.push(crate::ui::settings::row_status(
                t("migrate.clipboard"),
                found.or_else(|| Some(t("migrate.clipboard.desc"))),
                button("mg-paste", t("migrate.clipboard.button"))
                    .child(icon("copy", 14.0, white(0.6)))
                    .on_click(cx.listener(|this, _, _, cx| this.migrate_paste_clipboard(cx))),
            ));
            if takeout {
                let picked = (!m.files.is_empty()).then(|| tf("migrate.files.picked", &[("n", &m.files.len())]));
                let export = s.export_url();
                list.push(crate::ui::settings::row_status(
                    t("migrate.takeout"),
                    picked.or_else(|| Some(t("migrate.takeout.desc"))),
                    div()
                        .flex()
                        .gap(px(8.0))
                        .when_some(export, |d, url| {
                            d.child(text_button("mg-takeout", t("migrate.takeout.open")).text_size(theme::text_sm()).on_click(move |_, _, cx| cx.open_url(url)))
                        })
                        .child(
                            button("mg-files", t("migrate.files.button"))
                                .child(icon("folder-open", 14.0, white(0.6)))
                                .on_click(cx.listener(|this, _, _, cx| this.migrate_pick_files(cx))),
                        ),
                ));
            }
            m.pasted.is_some() || !m.files.is_empty()
        }
        Connect::Profile | Connect::UserName => {
            list.push(row(
                t(&format!("migrate.{}.profile", s.key())),
                Some(t(&format!("migrate.{}.profile.desc", s.key()))),
                input_box(m.profile.clone().unwrap()),
            ));
            true
        }
    };
    div()
        .flex()
        .flex_col()
        .child(section_label(s.name()))
        .child(div().pt(px(6.0)).text_size(px(13.0)).text_color(white(0.6)).child(connect_desc(s)))
        .child(rows(list))
        .child(
            div()
                .pt(px(16.0))
                .flex()
                .items_center()
                .gap(px(8.0))
                .child(back)
                .child(div().flex_1())
                .child(
                    primary_button("mg-go", t("migrate.next"))
                        .when(!ready, |b| b.opacity(0.5).cursor_default())
                        .when(ready, |b| b.on_click(cx.listener(|this, _, _, cx| this.migrate_connect(cx)))),
                ),
        )
        .child(note(t("migrate.privacy")))
}

fn collection_row(c: &Collection, on: bool, done_before: bool, cx: &mut Context<Glyph>) -> AnyElement {
    let id = c.id.clone();
    let key = format!("mg-col-{}", c.id);
    div()
        .id(SharedString::from(key.clone()))
        .flex()
        .items_center()
        .gap(px(10.0))
        .h(px(36.0))
        .px(px(10.0))
        .rounded(px(8.0))
        .cursor_pointer()
        .text_size(px(13.5))
        .relative()
        .child(hover_fill(key, white(0.05), 8.0))
        .on_click(cx.listener(move |this, _, _, cx| this.migrate_toggle(&id, cx)))
        .child(checkbox(on))
        .child(
            div()
                .min_w_0()
                .truncate()
                .font_weight(FontWeight::MEDIUM)
                .text_color(if on { gpui::white() } else { white(0.55) })
                .child(collection_name(c)),
        )
        .child(count_chip(c.count))
        .when(c.followed, |d| d.child(div().text_size(px(12.0)).text_color(white(0.45)).child(t("migrate.followed"))))
        .when(done_before, |d| d.child(div().text_size(px(12.0)).text_color(white(0.45)).child(t("migrate.done_before"))))
        .child(div().flex_1())
        .into_any_element()
}

fn choose_page(app: &mut Glyph, cx: &mut Context<Glyph>) -> Div {
    let Step::Choose(c) = &app.migrate.step else { return div() };
    let main: Vec<AnyElement> = c
        .overview
        .collections
        .iter()
        .filter(|x| x.kind != Kind::Playlist)
        .map(|x| collection_row(x, c.picked.contains(&x.id), c.done_before.contains(&x.id), cx))
        .collect();
    let lists: Vec<&Collection> = c.overview.collections.iter().filter(|x| x.kind == Kind::Playlist).collect();
    let n_lists = lists.len();
    let list_rows: Vec<AnyElement> = lists.iter().map(|x| collection_row(x, c.picked.contains(&x.id), c.done_before.contains(&x.id), cx)).collect();
    let picked = c.picked.len();
    let (service, account) = (c.service, c.overview.account.clone());
    div()
        .flex()
        .flex_col()
        .child(section_label(tf("migrate.account", &[("service", &service.name()), ("account", &account)])))
        .child(div().pt(px(6.0)).flex().flex_col().children(main))
        .when(n_lists > 0, |d| {
            d.child(
                div()
                    .flex()
                    .items_end()
                    .justify_between()
                    .child(section_label(tf("migrate.playlists", &[("n", &n_lists)])))
                    .child(
                        div()
                            .flex()
                            .gap(px(2.0))
                            .text_size(theme::text_sm())
                            .child(text_button("mg-all", t("migrate.all")).on_click(cx.listener(|this, _, _, cx| this.migrate_all_playlists(true, cx))))
                            .child(text_button("mg-none", t("migrate.none")).on_click(cx.listener(|this, _, _, cx| this.migrate_all_playlists(false, cx)))),
                    ),
            )
            .child(div().pt(px(6.0)).flex().flex_col().children(list_rows))
        })
        .child(
            div()
                .pt(px(18.0))
                .flex()
                .items_center()
                .gap(px(8.0))
                .child(text_button("mg-back2", t("migrate.back")).on_click(cx.listener(|this, _, _, cx| this.migrate_back(cx))))
                .child(div().flex_1())
                .child(
                    primary_button("mg-start", tf("migrate.start", &[("n", &picked)]))
                        .when(picked == 0, |b| b.opacity(0.5).cursor_default())
                        .when(picked > 0, |b| b.on_click(cx.listener(|this, _, _, cx| this.migrate_start(cx)))),
                ),
        )
        .child(note(t("migrate.choose.note")))
}

fn totals_line(x: &Totals) -> String {
    format!(
        "matched {}/{} ({}%) · {} liked · {} playlists · {} albums · {} artists",
        x.matched,
        x.songs,
        x.percent(),
        x.liked,
        x.playlists,
        x.albums,
        x.artists
    )
}

fn running_page(app: &mut Glyph, cx: &mut Context<Glyph>) -> Div {
    let Step::Running(r) = &app.migrate.step else { return div() };
    let p = glyph_core::ImportProgress { label: format!("IMPORT {}", r.service.key()), done: r.done, total: r.total };
    let stopping = r.cancel.load(Ordering::Relaxed);
    div()
        .flex()
        .flex_col()
        .gap(px(12.0))
        .child(section_label(tf("migrate.running", &[("service", &r.service.name())])))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(8.0))
                .rounded(px(10.0))
                .bg(white(0.035))
                .p(px(18.0))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.0))
                        .text_size(px(14.0))
                        .font_weight(FontWeight::MEDIUM)
                        .child(mono().text_color(white(0.5)).child(app.spinner().to_string()))
                        .child(tf("migrate.list", &[("i", &(r.index + 1)), ("n", &r.lists), ("name", &r.current)])),
                )
                .child(mono().text_size(px(12.5)).text_color(white(0.7)).child(if r.total > 0 { glyph_core::import::format_progress(&p, 24) } else { t("migrate.reading").to_string() }))
                .child(mono().text_size(px(12.0)).text_color(white(0.45)).child(totals_line(&r.totals))),
        )
        .child(
            div().flex().child(
                button("mg-stop", if stopping { t("migrate.stopping") } else { t("migrate.stop") })
                    .when(!stopping, |b| b.on_click(cx.listener(|this, _, _, cx| this.migrate_cancel(cx)))),
            ),
        )
        .child(note(t("migrate.running.note")))
}

fn done_page(app: &mut Glyph, cx: &mut Context<Glyph>) -> Div {
    let Step::Done(f) = &app.migrate.step else { return div() };
    let x = &f.totals;
    let mut items = vec![
        row(t("migrate.liked"), None, mono().text_color(white(0.8)).child(format!("+{}", x.liked))),
        row(t("migrate.playlists.done"), None, mono().text_color(white(0.8)).child(format!("+{}", x.playlists))),
        row(t("migrate.albums"), None, mono().text_color(white(0.8)).child(format!("+{}", x.albums))),
        row(t("migrate.artists"), None, mono().text_color(white(0.8)).child(format!("+{}", x.artists))),
    ];
    if x.unsure > 0 {
        items.push(row(t("migrate.unsure"), Some(t("migrate.unsure.desc")), mono().text_color(white(0.8)).child(x.unsure.to_string())));
    }
    let missing = x.songs.saturating_sub(x.matched);
    if missing > 0 {
        items.push(row(t("migrate.missing"), Some(t("migrate.missing.desc")), mono().text_color(white(0.8)).child(missing.to_string())));
    }
    let failed: Vec<AnyElement> = f.failed.iter().map(|(name, e)| row(name.clone(), None, div().max_w(px(360.0)).text_size(px(12.5)).text_color(white(0.55)).child(e.clone()))).collect();
    let notes: Vec<Div> = f.notes.iter().map(|k| note(t(&format!("migrate.note.{k}")))).collect();
    div()
        .flex()
        .flex_col()
        .child(section_label(if f.cancelled { tf("migrate.stopped.title", &[("service", &f.service.name())]) } else { tf("migrate.done", &[("service", &f.service.name())]) }))
        .child(div().pt(px(8.0)).child(mono().text_size(px(12.5)).text_color(white(0.7)).child(totals_line(x))))
        .child(rows(items))
        .when(!failed.is_empty(), |d| d.child(section_label(t("migrate.failed"))).child(rows(failed)))
        .children(notes)
        .child(
            div()
                .pt(px(18.0))
                .flex()
                .items_center()
                .gap(px(8.0))
                .child(text_button("mg-again", t("migrate.another")).on_click(cx.listener(|this, _, _, cx| this.migrate_back(cx))))
                .child(div().flex_1())
                .child(primary_button("mg-library", t("migrate.to_library")).on_click(cx.listener(|this, _, w, cx| {
                    this.close_settings(w, cx);
                    this.navigate(View::Library, w, cx);
                }))),
        )
}

/// Compact progress card for the library page while a move runs.
pub fn library_banner(app: &mut Glyph, cx: &mut Context<Glyph>) -> Option<AnyElement> {
    let r = app.migrate.running()?;
    let progress = if r.total > 0 { format!("{}/{}", r.done, r.total) } else { "…".into() };
    let label = tf("migrate.banner", &[("service", &r.service.name()), ("i", &(r.index + 1)), ("n", &r.lists)]);
    Some(
        div()
            .id("mg-banner")
            .flex()
            .items_center()
            .gap(px(10.0))
            .h(px(40.0))
            .px(px(14.0))
            .rounded(px(10.0))
            .bg(theme::card())
            .border_1()
            .border_color(theme::border())
            .cursor_pointer()
            .text_size(theme::text_sm())
            .hover(|s| s.bg(theme::card_hover()))
            .on_click(cx.listener(|this, _, _, cx| this.open_migrate(cx)))
            .child(icon("arrow-right-left", 14.0, white(0.8)))
            .child(div().text_color(white(0.9)).child(label))
            .child(div().min_w_0().truncate().text_color(white(0.5)).child(r.current.clone()))
            .child(div().flex_1())
            .child(mono().text_size(px(12.0)).text_color(white(0.6)).child(progress))
            .into_any_element(),
    )
}
