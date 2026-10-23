//! The encrypted wire form of library sync (see `e2e` for the keys).
//!
//! [`crate::sync`] keeps working on plain items. This layer turns them into
//! encrypted items right before a push and back right after a pull, and
//! remembers which form each item has on the server:
//!
//! * `plain`: a readable copy is on the server (old items, items from older
//!   apps, and playlists shown on the profile, which the server must read);
//! * `sealed`: an encrypted copy is on the server.
//!
//! A readable copy that has an encrypted one is replaced by a tombstone. Such
//! a tombstone never deletes anything here, because the encrypted copy is the
//! real one. Playlists keep the server's revision check per form.

use crate::e2e::{self, DataKey, OpenError};
use crate::sync::{Key, Snapshot, SyncState};
use glyph_protocol::{sync_kind as kind, SyncItem};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

/// Encryption state of one account on this device, saved with the sync state.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct E2eState {
    /// Format of this state (see `account::E2E_VERSION`).
    #[serde(default)]
    pub version: u32,
    /// Keyring entries as the server last sent them.
    #[serde(default)]
    pub keyring: BTreeMap<String, Value>,
    /// A keyring existed once: one that disappears later was removed by the server.
    #[serde(default)]
    pub keyring_seen: bool,
    /// Encrypted items this device could not open yet, by their wire key.
    #[serde(default)]
    pub locked: BTreeMap<String, SyncItem>,
    /// `kind \u{1f} key` with a readable copy on the server.
    #[serde(default)]
    pub plain: BTreeSet<String>,
    /// `kind \u{1f} key` with an encrypted copy on the server.
    #[serde(default)]
    pub sealed: BTreeSet<String>,
    /// Came in under an older data key: push again with the current one.
    #[serde(default)]
    pub reseal: BTreeSet<String>,
    /// Playlist ids shown on the profile (stay readable for the server).
    #[serde(default)]
    pub public: BTreeSet<String>,
    /// When the profile's playlists were last read (ms).
    #[serde(default)]
    pub public_checked: u64,
    /// Server revision of each playlist's readable / encrypted copy.
    #[serde(default)]
    pub plain_revs: BTreeMap<String, u64>,
    #[serde(default)]
    pub sealed_revs: BTreeMap<String, u64>,
    /// Data keys that were ever stored in the clear (`escrow`): the server
    /// could read what they encrypted.
    #[serde(default)]
    pub escrowed: BTreeSet<String>,
}

pub fn joined(key: &Key) -> String {
    format!("{}\u{1f}{}", key.0, key.1)
}

fn split(s: &str) -> Option<Key> {
    s.split_once('\u{1f}').map(|(a, b)| (a.to_string(), b.to_string()))
}

fn is_wire_encrypted(k: &str) -> bool {
    k == kind::E2E || k == kind::E2E_PLAYLIST
}

/// The keyring's `main` entry.
#[derive(Debug, Clone, PartialEq)]
pub struct Main {
    pub kid: String,
    pub pw: Option<Value>,
    pub escrow: Option<DataKey>,
}

impl Main {
    pub fn parse(v: &Value) -> Option<Self> {
        let kid = v["kid"].as_str()?.to_string();
        let escrow = v["escrow"].as_str().and_then(DataKey::from_b64).filter(|k| k.kid() == kid);
        Some(Self { kid, pw: v.get("pw").filter(|p| p.is_object()).cloned(), escrow })
    }

    pub fn value(k: &DataKey, pw: Option<Value>, escrow: bool) -> Value {
        let mut v = json!({ "v": 1, "kid": k.kid() });
        if let Some(pw) = pw {
            v["pw"] = pw;
        }
        if escrow {
            v["escrow"] = json!(k.to_b64());
        }
        v
    }
}

impl E2eState {
    pub fn main(&self) -> Option<Main> {
        self.keyring.get(e2e::MAIN).and_then(Main::parse)
    }

    /// Takes the keyring entries out of pulled items.
    pub fn absorb_keyring(&mut self, items: &[SyncItem]) {
        for i in items.iter().filter(|i| i.kind == kind::KEYRING) {
            match i.value.as_ref().filter(|_| !i.deleted) {
                Some(v) => {
                    self.keyring.insert(i.key.clone(), v.clone());
                }
                None => {
                    self.keyring.remove(&i.key);
                }
            }
        }
        if self.keyring.contains_key(e2e::MAIN) {
            self.keyring_seen = true;
        }
    }

    /// Readable copies of these items are still on the server.
    pub fn mark_plain(&mut self, keys: impl IntoIterator<Item = String>) {
        self.plain.extend(keys);
    }
}

pub fn keyring_item(key: &str, value: Option<Value>, now: u64) -> SyncItem {
    SyncItem { kind: kind::KEYRING.into(), key: key.into(), deleted: value.is_none(), value, updated_at: now, rev: 0 }
}

/// What a pushed item does to the state once the server accepted it.
#[derive(Debug, Clone, PartialEq)]
pub enum Effect {
    /// One of the plain items from the outbox: mark it pushed.
    Plain(SyncItem),
    PlainUp(String),
    PlainGone(String),
    SealedUp(String),
    SealedGone(String),
    None,
}

/// One item on the wire and what accepting it means.
#[derive(Debug, Clone)]
pub struct Wire {
    pub item: SyncItem,
    pub effects: Vec<Effect>,
}

/// Plain items whose server copy has the wrong form: readable although it
/// should be encrypted (or the reverse for profile playlists), or encrypted
/// with an older key. Their current value comes from `snap`.
pub fn repush(st: &SyncState, snap: &Snapshot, now: u64) -> Vec<SyncItem> {
    let e = &st.e2e;
    let map: BTreeMap<String, &Value> = snap.items.iter().map(|(k, v)| (joined(k), v)).collect();
    let mut wanted: BTreeSet<String> = e.reseal.clone();
    for j in &e.plain {
        let Some((k, key)) = split(j) else { continue };
        let public = k == kind::PLAYLIST && e.public.contains(&key);
        if !public {
            wanted.insert(j.clone());
        }
    }
    for id in &e.public {
        let j = joined(&(kind::PLAYLIST.to_string(), id.clone()));
        if e.sealed.contains(&j) || !e.plain.contains(&j) {
            wanted.insert(j);
        }
    }
    // Only what this device has: another device's statistics or keys it
    // cannot open stay for the device they belong to (a deletion made here
    // retires both copies through the outbox).
    wanted
        .into_iter()
        .filter_map(|j| {
            let v = map.get(&j)?;
            let key = split(&j)?;
            Some(SyncItem { kind: key.0, key: key.1, value: Some((*v).clone()), updated_at: now, deleted: false, rev: 0 })
        })
        .collect()
}

/// Plain items -> what goes on the wire. `keys[0]` encrypts; copies under
/// older keys (`keys[1..]`) of re-encrypted items are retired.
pub fn outgoing(items: Vec<SyncItem>, keys: &[DataKey], e: &E2eState) -> Vec<Wire> {
    let Some(k) = keys.first() else { return Vec::new() };
    let mut out = Vec::new();
    let mut seen = BTreeSet::new();
    for it in items {
        let j = joined(&(it.kind.clone(), it.key.clone()));
        // An outbox item and a re-push of the same key: send it once.
        if !seen.insert(j.clone()) {
            continue;
        }
        let playlist = it.kind == kind::PLAYLIST;
        if playlist && !it.deleted && e.public.contains(&it.key) {
            // Shown on the profile: the server reads it, so it goes readable.
            let mut plain = it.clone();
            plain.rev = e.plain_revs.get(&it.key).copied().unwrap_or(0);
            if let Some(v) = plain.value.as_mut().and_then(Value::as_object_mut) {
                v.insert("public".into(), json!(true));
            }
            out.push(Wire { item: plain, effects: vec![Effect::Plain(it.clone()), Effect::PlainUp(j.clone())] });
            if e.sealed.contains(&j) {
                let mut gone = it.clone();
                gone.deleted = true;
                gone.value = None;
                gone.rev = e.sealed_revs.get(&it.key).copied().unwrap_or(0);
                out.push(Wire { item: k.seal_item(&gone), effects: vec![Effect::SealedGone(j)] });
            }
            continue;
        }
        let mut sealed_src = it.clone();
        if playlist {
            sealed_src.rev = e.sealed_revs.get(&it.key).copied().unwrap_or(0);
        }
        let effect = if it.deleted { Effect::SealedGone(j.clone()) } else { Effect::SealedUp(j.clone()) };
        out.push(Wire { item: k.seal_item(&sealed_src), effects: vec![Effect::Plain(it.clone()), effect] });
        if e.plain.contains(&j) {
            // Retire the readable copy after the encrypted one is in.
            let rev = if playlist { e.plain_revs.get(&it.key).copied().unwrap_or(0) } else { 0 };
            let gone = SyncItem { kind: it.kind.clone(), key: it.key.clone(), value: None, updated_at: it.updated_at, deleted: true, rev };
            out.push(Wire { item: gone, effects: vec![Effect::PlainGone(j.clone())] });
        }
        // Copies under an older key (playlists keep theirs: their revision
        // under the old key is unknown here).
        if e.reseal.contains(&j) && !playlist {
            for old in &keys[1..] {
                let gone = SyncItem { kind: it.kind.clone(), key: it.key.clone(), value: None, updated_at: it.updated_at, deleted: true, rev: 0 };
                let mut w = k.seal_item(&gone);
                w.key = old.blind(&it.kind, &it.key);
                out.push(Wire { item: w, effects: vec![Effect::None] });
            }
        }
    }
    out
}

/// Items without encryption (no keyring yet), with the same bookkeeping.
pub fn plain_wires(items: Vec<SyncItem>) -> Vec<Wire> {
    items
        .into_iter()
        .map(|it| {
            let j = joined(&(it.kind.clone(), it.key.clone()));
            let form = if it.deleted { Effect::PlainGone(j) } else { Effect::PlainUp(j) };
            Wire { effects: vec![Effect::Plain(it.clone()), form], item: it }
        })
        .collect()
}

/// Wires in push batches (an item and the tombstone of its old copy stay together).
pub fn batches(wires: Vec<Wire>, max_items: usize, max_bytes: usize) -> Vec<Vec<Wire>> {
    let mut out: Vec<Vec<Wire>> = Vec::new();
    let mut cur: Vec<Wire> = Vec::new();
    let mut size = 0;
    for w in wires {
        let n = serde_json::to_vec(&w.item).map(|b| b.len()).unwrap_or(0);
        let follows = w.effects.iter().all(|e| matches!(e, Effect::PlainGone(_) | Effect::SealedGone(_) | Effect::None)) && w.item.deleted;
        if !cur.is_empty() && !follows && (cur.len() >= max_items || size + n > max_bytes) {
            out.push(std::mem::take(&mut cur));
            size = 0;
        }
        size += n;
        cur.push(w);
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// Records what the server accepted.
pub fn accepted(st: &mut SyncState, wires: &[Wire]) {
    let mut plain_items = Vec::new();
    for w in wires {
        for e in &w.effects {
            match e {
                Effect::Plain(i) => plain_items.push(i.clone()),
                Effect::PlainUp(j) => {
                    st.e2e.plain.insert(j.clone());
                }
                Effect::PlainGone(j) => {
                    st.e2e.plain.remove(j);
                }
                Effect::SealedUp(j) => {
                    st.e2e.sealed.insert(j.clone());
                    st.e2e.reseal.remove(j);
                }
                Effect::SealedGone(j) => {
                    st.e2e.sealed.remove(j);
                    st.e2e.reseal.remove(j);
                }
                Effect::None => {}
            }
        }
    }
    st.mark_pushed(&plain_items);
}

/// Something about the encrypted items that the user should know.
#[derive(Debug, Clone, PartialEq)]
pub enum Notice {
    /// An encrypted item did not authenticate: changed on the server.
    Tampered,
}

/// Pulled (or stale) items -> plain items for [`crate::sync::apply`].
/// `keys` are the current data key and older ones. Encrypted items no key
/// opens wait in `locked`.
pub fn incoming(items: Vec<SyncItem>, keys: &[DataKey], st: &mut SyncState, notices: &mut Vec<Notice>) -> Vec<SyncItem> {
    // Blinded key -> (plain key, which data key), for encrypted tombstones.
    let mut index: BTreeMap<String, (Key, usize)> = BTreeMap::new();
    if !keys.is_empty() && items.iter().any(|i| is_wire_encrypted(&i.kind) && i.deleted) {
        let all: BTreeSet<String> = st.known.keys().chain(st.e2e.sealed.iter()).chain(st.e2e.plain.iter()).chain(st.outbox.keys()).cloned().collect();
        for j in all {
            let Some(key) = split(&j) else { continue };
            for (n, k) in keys.iter().enumerate().rev() {
                index.insert(k.blind(&key.0, &key.1), (key.clone(), n));
            }
        }
    }
    let mut out = Vec::new();
    for it in items {
        if it.kind == kind::KEYRING {
            continue;
        }
        if is_wire_encrypted(&it.kind) {
            if keys.is_empty() {
                if !it.deleted {
                    st.e2e.locked.insert(format!("{}\u{1f}{}", it.kind, it.key), it);
                }
                continue;
            }
            let opened = if it.deleted || it.value.is_none() {
                match index.get(&it.key) {
                    // A copy under an older key was retired: the current one stays.
                    Some((_, n)) if *n > 0 => continue,
                    Some((key, _)) => {
                        let key = key.clone();
                        keys[0].open_item(&it, |_| Some(key.clone())).ok().flatten().map(|p| (p, false))
                    }
                    // A tombstone of something this device never had.
                    None => continue,
                }
            } else {
                let mut found = None;
                let mut tampered = false;
                for (n, k) in keys.iter().enumerate() {
                    match k.open_item(&it, |_| None) {
                        Ok(o) => {
                            found = o.map(|p| (p, n > 0));
                            break;
                        }
                        Err(OpenError::OtherKey(_)) => {}
                        Err(OpenError::Tampered) => {
                            tampered = true;
                            break;
                        }
                    }
                }
                if tampered {
                    notices.push(Notice::Tampered);
                    continue;
                }
                if found.is_none() {
                    st.e2e.locked.insert(format!("{}\u{1f}{}", it.kind, it.key), it);
                    continue;
                }
                found
            };
            let Some((plain, old)) = opened else { continue };
            let j = joined(&(plain.kind.clone(), plain.key.clone()));
            let playlist = plain.kind == kind::PLAYLIST;
            if playlist {
                st.e2e.sealed_revs.insert(plain.key.clone(), it.rev);
            }
            if plain.deleted {
                st.e2e.sealed.remove(&j);
                // The playlist went on the profile: its readable copy is the real one.
                if playlist && st.e2e.public.contains(&plain.key) && st.e2e.plain.contains(&j) {
                    continue;
                }
            } else {
                st.e2e.sealed.insert(j.clone());
                if playlist {
                    st.e2e.public.remove(&plain.key);
                }
                if old {
                    st.e2e.reseal.insert(j);
                }
            }
            out.push(plain);
            continue;
        }
        let j = joined(&(it.kind.clone(), it.key.clone()));
        let playlist = it.kind == kind::PLAYLIST;
        // Without the key this device cannot tell a retired readable copy
        // from a deletion: the tombstone waits, in order, with the
        // encrypted items.
        if keys.is_empty() && (it.deleted || it.value.is_none()) && st.e2e.keyring.contains_key(e2e::MAIN) {
            st.e2e.locked.insert(j, it);
            continue;
        }
        if playlist {
            st.e2e.plain_revs.insert(it.key.clone(), it.rev);
        }
        if it.deleted || it.value.is_none() {
            st.e2e.plain.remove(&j);
            // A readable copy retired after its encrypted copy went up.
            if st.e2e.sealed.contains(&j) {
                continue;
            }
        } else {
            st.e2e.plain.insert(j.clone());
            if playlist && it.value.as_ref().is_some_and(|v| v["public"] == json!(true)) {
                st.e2e.public.insert(it.key.clone());
            }
        }
        let mut it = it;
        if let Some(v) = it.value.as_mut().and_then(Value::as_object_mut) {
            if playlist {
                v.remove("public");
            }
        }
        out.push(it);
    }
    out
}

/// Encrypted items that waited for a key, oldest first, ready for [`incoming`].
pub fn take_locked(st: &mut SyncState) -> Vec<SyncItem> {
    let mut v: Vec<SyncItem> = std::mem::take(&mut st.e2e.locked).into_values().collect();
    v.sort_by_key(|i| i.rev);
    v
}

/// How this device gets the data key, given the keyring.
#[derive(Debug, Clone, PartialEq)]
pub enum Source {
    Escrow,
    Password,
    Device,
    Pairing,
    /// A rotated key, proven by the previous one.
    Rotation,
}

/// Inputs for [`choose`].
pub struct Choice<'a> {
    pub account_id: &'a str,
    pub device_id: &'a str,
    pub current: Option<&'a DataKey>,
    pub pw_key: Option<&'a [u8; 32]>,
    pub device: &'a e2e::DeviceKey,
    /// The official server: its sign-in page is trusted anyway, so keys it
    /// relays between devices are too.
    pub official: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Chosen {
    /// The current key is the keyring's.
    Keep,
    /// Switch to this key.
    Take(DataKey, Source),
    /// The keyring has a key this device cannot open (yet).
    Locked,
    /// The server replaced the keyring with a key it can read although this
    /// device holds the real one: refused.
    Refused,
    /// No keyring on the server.
    NoKeyring,
}

/// Picks the data key from the keyring (pairing entries are handled by the caller).
pub fn choose(e: &E2eState, c: &Choice) -> Chosen {
    let Some(main) = e.main() else { return Chosen::NoKeyring };
    if c.current.is_some_and(|k| k.kid() == main.kid) {
        return Chosen::Keep;
    }
    if let Some(pw) = c.pw_key {
        if let Some(k) = main.pw.as_ref().and_then(|v| e2e::unwrap_with_password(pw, c.account_id, &main.kid, v)) {
            return Chosen::Take(k, Source::Password);
        }
    }
    let env = e.keyring.get(&format!("{}{}", e2e::ENV, c.device_id)).filter(|v| v["kid"].as_str() == Some(main.kid.as_str()));
    let opened = env.and_then(|v| e2e::open_from_device(c.device_id, c.device, v).map(|k| (k, v)));
    match c.current {
        None => {
            if let Some(k) = main.escrow.clone() {
                return Chosen::Take(k, Source::Escrow);
            }
            // Nothing here proves who sealed it, except the trusted server.
            if let (true, Some((k, _))) = (c.official, opened) {
                return Chosen::Take(k, Source::Device);
            }
            Chosen::Locked
        }
        Some(old) => {
            if let Some((k, v)) = opened {
                // A rotation from a key the server never had, or the trusted server.
                if !e.escrowed.contains(&old.kid()) && e2e::rotation_proven(v, c.device_id, old) {
                    return Chosen::Take(k, Source::Rotation);
                }
                if c.official {
                    return Chosen::Take(k, Source::Device);
                }
            }
            if let Some(k) = main.escrow.clone() {
                // Both keys were readable by the server anyway (two devices
                // created a keyring at the same time): take the one that won.
                if e.escrowed.contains(&old.kid()) {
                    return Chosen::Take(k, Source::Escrow);
                }
                // The server put a key in the clear over ours.
                return Chosen::Refused;
            }
            Chosen::Locked
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(k: &str, key: &str, v: Option<Value>, rev: u64) -> SyncItem {
        SyncItem { kind: k.into(), key: key.into(), deleted: v.is_none(), value: v, updated_at: 1, rev }
    }

    fn j(k: &str, key: &str) -> String {
        joined(&(k.to_string(), key.to_string()))
    }

    #[test]
    fn migration_replaces_readable_copies_and_their_tombstones_delete_nothing() {
        let k = DataKey::generate();
        let mut st = SyncState::default();
        let mut notices = Vec::new();
        // Old readable items arrive and are applied as they are.
        let pulled = incoming(vec![item("liked", "a", Some(json!({"t": 1})), 1)], &[k.clone()], &mut st, &mut notices);
        assert_eq!(pulled.len(), 1);
        assert!(st.e2e.plain.contains(&j("liked", "a")));
        let snap = Snapshot { items: vec![(("liked".into(), "a".into()), json!({"t": 1}))], ..Default::default() };
        let again = repush(&st, &snap, 5);
        assert_eq!(again.len(), 1);
        let wires = outgoing(again, &[k.clone()], &st.e2e);
        assert_eq!(wires.len(), 2);
        assert_eq!(wires[0].item.kind, kind::E2E);
        assert!(wires[1].item.deleted && wires[1].item.kind == "liked");
        accepted(&mut st, &wires);
        assert!(st.e2e.sealed.contains(&j("liked", "a")) && !st.e2e.plain.contains(&j("liked", "a")));
        assert!(repush(&st, &snap, 6).is_empty());
        // Another device pulls both: it keeps the song.
        let mut other = SyncState::default();
        other.e2e.plain.insert(j("liked", "a"));
        other.known.insert(j("liked", "a"), "x".into());
        let echo: Vec<SyncItem> = wires.iter().map(|w| w.item.clone()).collect();
        let got = incoming(echo, &[k.clone()], &mut other, &mut notices);
        assert_eq!(got.len(), 1);
        assert!(!got[0].deleted);
        assert!(notices.is_empty());
    }

    #[test]
    fn deletions_from_older_apps_still_apply() {
        let k = DataKey::generate();
        let mut st = SyncState::default();
        st.e2e.plain.insert(j("liked", "a"));
        let got = incoming(vec![item("liked", "a", None, 4)], &[k], &mut st, &mut Vec::new());
        assert_eq!(got.len(), 1);
        assert!(got[0].deleted);
    }

    #[test]
    fn profile_playlists_stay_readable_and_switch_back() {
        let k = DataKey::generate();
        let mut st = SyncState::default();
        st.e2e.public.insert("p1".into());
        st.e2e.sealed.insert(j("playlist", "p1"));
        st.e2e.sealed_revs.insert("p1".into(), 9);
        let snap = Snapshot { items: vec![(("playlist".into(), "p1".into()), json!({"name": "Mix", "tracks": []}))], ..Default::default() };
        let wires = outgoing(repush(&st, &snap, 1), &[k.clone()], &st.e2e);
        assert_eq!(wires[0].item.kind, "playlist");
        assert_eq!(wires[0].item.value.as_ref().unwrap()["public"], json!(true));
        assert_eq!(wires[1].item.kind, kind::E2E_PLAYLIST);
        assert!(wires[1].item.deleted);
        assert_eq!(wires[1].item.rev, 9);
        accepted(&mut st, &wires);
        assert!(repush(&st, &snap, 2).is_empty());
        // Taken off the profile: encrypted again, readable copy retired.
        st.e2e.public.clear();
        st.e2e.plain_revs.insert("p1".into(), 11);
        let wires = outgoing(repush(&st, &snap, 3), &[k.clone()], &st.e2e);
        assert_eq!(wires[0].item.kind, kind::E2E_PLAYLIST);
        assert_eq!((wires[1].item.kind.as_str(), wires[1].item.deleted, wires[1].item.rev), ("playlist", true, 11));
    }

    #[test]
    fn locked_items_wait_for_the_key() {
        let k = DataKey::generate();
        let sealed = k.seal_item(&item("liked", "a", Some(json!(1)), 0));
        let mut st = SyncState::default();
        assert!(incoming(vec![sealed], &[], &mut st, &mut Vec::new()).is_empty());
        assert_eq!(st.e2e.locked.len(), 1);
        let waiting = take_locked(&mut st);
        let got = incoming(waiting, &[k], &mut st, &mut Vec::new());
        assert_eq!(got[0].key, "a");
    }

    #[test]
    fn a_device_without_the_key_keeps_items_through_the_migration() {
        let k = DataKey::generate();
        let mut st = SyncState::default();
        st.e2e.keyring.insert(e2e::MAIN.into(), Main::value(&k, None, false));
        st.e2e.plain.insert(j("liked", "a"));
        st.known.insert(j("liked", "a"), "x".into());
        let mut sealed = k.seal_item(&item("liked", "a", Some(json!(1)), 0));
        sealed.rev = 5;
        let pulled = vec![sealed, item("liked", "a", None, 6)];
        assert!(incoming(pulled, &[], &mut st, &mut Vec::new()).is_empty());
        let got = incoming(take_locked(&mut st), &[k], &mut st, &mut Vec::new());
        assert_eq!(got.len(), 1);
        assert!(!got[0].deleted);
    }

    #[test]
    fn older_keys_open_and_mark_for_resealing() {
        let (old, new) = (DataKey::generate(), DataKey::generate());
        let sealed = old.seal_item(&item("liked", "a", Some(json!(1)), 0));
        let mut st = SyncState::default();
        let got = incoming(vec![sealed], &[new, old], &mut st, &mut Vec::new());
        assert_eq!(got.len(), 1);
        assert!(st.e2e.reseal.contains(&j("liked", "a")));
    }

    #[test]
    fn tampered_items_are_reported() {
        let k = DataKey::generate();
        let mut sealed = k.seal_item(&item("liked", "a", Some(json!(1)), 0));
        sealed.key = k.blind("liked", "b");
        let mut notices = Vec::new();
        let mut st = SyncState::default();
        assert!(incoming(vec![sealed], &[k], &mut st, &mut notices).is_empty());
        assert_eq!(notices, vec![Notice::Tampered]);
    }

    fn with_main(k: &DataKey, pw: Option<Value>, escrow: bool) -> E2eState {
        let mut e = E2eState::default();
        e.keyring.insert(e2e::MAIN.into(), Main::value(k, pw, escrow));
        e
    }

    #[test]
    fn keys_come_from_escrow_password_or_trusted_devices() {
        let k = DataKey::generate();
        let dk = e2e::DeviceKey::generate();
        let c = |current, pw_key, official| Choice { account_id: "acc", device_id: "d1", current, pw_key, device: &dk, official };
        assert_eq!(choose(&E2eState::default(), &c(None, None, false)), Chosen::NoKeyring);
        assert_eq!(choose(&with_main(&k, None, true), &c(None, None, false)), Chosen::Take(k.clone(), Source::Escrow));
        assert_eq!(choose(&with_main(&k, None, true), &c(Some(&k), None, false)), Chosen::Keep);
        let pw = [3u8; 32];
        let e = with_main(&k, Some(e2e::wrap_with_password(&pw, "acc", &k)), false);
        assert_eq!(choose(&e, &c(None, Some(&pw), false)), Chosen::Take(k.clone(), Source::Password));
        assert_eq!(choose(&e, &c(None, None, false)), Chosen::Locked);
        // Sealed to this device: only the official server is trusted for it.
        let mut e = with_main(&k, None, false);
        e.keyring.insert(format!("{}d1", e2e::ENV), e2e::seal_to_device("d1", &dk.public_b64(), &k, "d0", None).unwrap());
        assert_eq!(choose(&e, &c(None, None, false)), Chosen::Locked);
        assert_eq!(choose(&e, &c(None, None, true)), Chosen::Take(k.clone(), Source::Device));
    }

    #[test]
    fn rotations_need_proof_and_escrow_cannot_replace_a_key() {
        let (old, new) = (DataKey::generate(), DataKey::generate());
        let dk = e2e::DeviceKey::generate();
        let c = |current| Choice { account_id: "acc", device_id: "d1", current, pw_key: None, device: &dk, official: false };
        let mut e = with_main(&new, None, false);
        e.keyring.insert(format!("{}d1", e2e::ENV), e2e::seal_to_device("d1", &dk.public_b64(), &new, "d0", Some(&old)).unwrap());
        assert_eq!(choose(&e, &c(Some(&old))), Chosen::Take(new.clone(), Source::Rotation));
        // The old key was readable by the server: its proof means nothing.
        e.escrowed.insert(old.kid());
        assert_eq!(choose(&e, &c(Some(&old))), Chosen::Locked);
        // A server swapping in a key of its own, in the clear.
        let forged = with_main(&DataKey::generate(), None, true);
        assert_eq!(choose(&forged, &c(Some(&old))), Chosen::Refused);
    }
}
