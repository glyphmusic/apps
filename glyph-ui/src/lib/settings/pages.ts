// The settings pages (settings/modal.rs page_info, subtabs, OPTIONS and the
// groups of settings/nav.rs). Shared by the desktop modal and the phone
// settings: a page body is a component with an optional `sub` prop (the
// selected sub-tab); on desktop the modal keeps the sub-tab in `sui.sub`.
import type { Component } from "svelte";
import { app, type SettingsPage } from "../state.svelte";
import { t } from "../i18n.svelte";
import { pl } from "./plugins-state.svelte";
import { onWeb } from "../web";
import { isMobile, onPhone } from "./util";
import { adv, pageAdvanced } from "./advanced.svelte";
import Overview from "./Overview.svelte";
import Playback from "./Playback.svelte";
import Audio from "./Audio.svelte";
import Appearance from "./Appearance.svelte";
import Server from "./Server.svelte";
import Account from "./Account.svelte";
import Storage from "./Storage.svelte";
import Transfer from "./Transfer.svelte";
import Migrate from "./Migrate.svelte";
import Integrations from "./Integrations.svelte";
import Plugins from "./Plugins.svelte";
import About from "./About.svelte";
import System from "./System.svelte";
import Notifications from "./Notifications.svelte";

/** Every page, in the order of the nav (nav.rs groups: preferences, then the app). */
export const PAGES: SettingsPage[] = [
  "overview", "playback", "audio", "appearance",
  "account", "server", "system", "notifications", "integrations", "plugins", "storage", "migrate", "transfer", "about",
];

/** Nav groups: label key and pages (nav.rs `groups`). */
export const GROUPS: { label: string; pages: SettingsPage[] }[] = [
  { label: "", pages: ["overview"] },
  { label: "sp.group.prefs", pages: ["playback", "audio", "appearance"] },
  { label: "sp.group.app", pages: ["account", "server", "system", "notifications", "integrations", "plugins", "storage", "migrate", "transfer", "about"] },
];

/** Extra words the nav search finds a page by (nav.rs `groups`). */
export const PAGE_WORDS: Record<SettingsPage, string> = {
  overview: "overview übersicht schnell quick wichtig important home allgemein general",
  playback: "sources source quelle youtube soundcloud priority radio quality qualität audio player mausrad wheel zen hotkey taste geschmack mixes recommendations empfehlungen collaborative server",
  audio: "equalizer eq bass treble höhen tiefen klang sound preset teilen share import export code datei file",
  appearance: "language sprache theme ascii cover glow visualizer textgröße text size",
  account: "account konto login sign in anmelden sync devices geräte handle passkey profile profil privacy privatsphäre picture bild banner public öffentlich",
  server: "url api key keys schlüssel",
  system: "system autostart startup start boot tray systray hintergrund background",
  notifications: "notifications benachrichtigungen mitteilungen push update version release releases neu new artist künstler follow folgen",
  integrations: "discord rich presence integration integrations status activity listening mcp ai assistant claude",
  plugins: "plugin plugins erweiterungen extensions addons themes github community",
  storage: "cache download downloads speicher ordner folder lokal local",
  migrate: "transfer übertragen umzug move import spotify youtube soundcloud deezer last.fm lastfm apple bandcamp tidal library bibliothek playlists",
  transfer: "backup export import share teilen code datei file",
  about: "update version support hilfe help",
};

const INFO: Record<SettingsPage, [string, string, string]> = {
  overview: ["sp.overview", "sp.overview.sub", "sliders-horizontal"],
  playback: ["sp.playback", "sp.playback.sub", "play"],
  audio: ["sp.eq", "sp.eq.sub", "volume-2"],
  appearance: ["sp.appearance", "sp.appearance.sub", "palette"],
  account: ["sp.account", "sp.account.sub", "user-round"],
  server: ["sp.server", "sp.server.sub", "server"],
  storage: ["sp.storage", "sp.storage.sub", "database"],
  transfer: ["sp.transfer", "sp.transfer.sub", "copy"],
  migrate: ["sp.migrate", "sp.migrate.sub", "arrow-right-left"],
  integrations: ["sp.integrations", "sp.integrations.sub", "link-2"],
  plugins: ["sp.plugins", "sp.plugins.sub", "puzzle"],
  about: ["sp.about", "sp.about.sub", "info"],
  system: ["sp.system", "sp.system.sub", "monitor"],
  notifications: ["sp.notify", "sp.notify.sub", "bell"],
};

/** Title, subtitle and nav icon of a page (modal.rs page_info). */
export function pageInfo(p: SettingsPage): { name: string; sub: string; icon: string } {
  const [name, sub, icon] = INFO[p];
  return { name: t(name), sub: t(sub), icon };
}

/** Sub-tabs of a page (label keys); empty: the page has none (modal.rs subtabs). */
export function subtabs(p: SettingsPage): string[] {
  const mobile = isMobile();
  switch (p) {
    // Web player: sound output is the app's (Playback.svelte maps the tabs back).
    // Mobile: no desktop Now Playing toggles (sidebar lyrics/download/shortcuts).
    case "playback":
      if (onWeb()) return mobile ? ["sp.sub.content", "settings.sources"] : ["sp.sub.content", "settings.sources", "sp.sub.player"];
      return mobile ? ["sp.sub.general", "sp.sub.content", "settings.sources"] : ["sp.sub.general", "sp.sub.content", "settings.sources", "sp.sub.player"];
    case "account": return app.account ? ["sp.sub.overview", "sp.sub.profile", "sp.sub.devices", "account.security"] : [];
    case "server": return ["sp.sub.connection", "settings.keys", "privacy.tab"];
    // Phones: only AI assistants (no Discord there).
    case "integrations": return onWeb() || onPhone() ? [] : ["sp.discord.section", "sp.mcp.section"];
    // Installed, Browse, then one tab per plugin with its own page.
    case "plugins": return ["plugins.tab.installed", "plugins.tab.browse", ...pl.ui.pages.map((x) => `plugin-page.${x.id}`)];
    case "about": return mobile ? ["sp.sub.version", "sp.support"] : ["sp.sub.version", "sp.support", "sp.sub.shortcuts"];
    default: return [];
  }
}

/** The component that renders a page body; it takes an optional `sub` prop. */
export function pageComponent(p: SettingsPage): Component<{ sub?: number }> {
  return COMPONENTS[p];
}
const COMPONENTS: Record<SettingsPage, Component<{ sub?: number }>> = {
  overview: Overview, playback: Playback, audio: Audio, appearance: Appearance, server: Server, account: Account,
  storage: Storage, transfer: Transfer, migrate: Migrate, integrations: Integrations, plugins: Plugins, about: About, system: System,
  notifications: Notifications,
};

/** A page is shown at all (phones have no autostart/tray or integrations; Web player shows a smaller set). */
export function pageShown(p: SettingsPage): boolean {
  if (!adv.on && pageAdvanced(p)) return false;
  // The web player has no equalizer, server, storage, plugins or system settings.
  if (onWeb()) {
    if (p === "integrations") return app.account != null;
    return p === "overview" || p === "account" || p === "playback" || p === "appearance" || p === "transfer" || p === "about";
  }
  // Phones show integrations for the AI assistant settings only.
  if (p === "integrations") return onPhone() || !isMobile();
  if (p === "system") return !isMobile();
  // Only the Android app shows notifications of its own.
  if (p === "notifications") return app.platform === "android";
  return true;
}

/** Every option the settings search finds on its own: page, sub-tab, label key
 *  (the row's label, so the row can light up), extra search words (modal.rs OPTIONS). */
export const OPTIONS: [SettingsPage, number, string, string][] = [
  ["notifications", 0, "sp.notify.updates", "update version neue new benachrichtigung notification"],
  ["notifications", 0, "sp.notify.releases", "release releases neu new artist künstler folgen follow benachrichtigung notification"],
  ["playback", 0, "sp.audio.quality", "streaming quality qualität bitrate"],
  ["server", 2, "privacy.title", "privacy datenschutz transparency transparenz encryption verschlüsselung e2e end-to-end ende-zu-ende server sees sieht"],
  ["playback", 0, "sp.audio.dl_quality", "download quality qualität flac"],
  ["playback", 0, "sp.crossfade", "crossfade überblenden fade"],
  ["playback", 0, "sp.output", "output ausgabe device gerät speaker lautsprecher headphones kopfhörer"],
  ["playback", 0, "sp.level", "volume lautstärke level"],
  ["playback", 0, "sp.normalize", "normalize normalisieren loudness lautheit"],
  ["playback", 0, "sp.devices", "network netzwerk devices geräte chromecast cast nest google upnp dlna tv fernseher sonos airplay mpd receiver lautsprecher speaker"],
  ["playback", 0, "sp.audio.channels", "mono stereo spatial surround 5.1 7.1 hrtf binaural 3d räumlich raumklang channels kanäle"],
  ["playback", 1, "sp.shuffle", "shuffle zufall random smart"],
  ["playback", 1, "sp.stats", "stats statistics statistik analytics hörzeit listening time tracking"],
  ["playback", 2, "sp.extractor", "r-dlp rdlp yt-dlp extractor youtube schneller faster"],
  ["playback", 2, "sp.search.mode", "search suche lokal local server rate limit ban quellen sources"],
  ["playback", 1, "sp.explicit", "explicit explizit"],
  ["playback", 1, "sp.radio", "radio autoplay"],
  ["playback", 1, "sp.taste", "taste geschmack algorithmus algorithm personalization personalisierung mixes daily mix"],
  ["playback", 1, "sp.taste_server", "collaborative server taste geschmack privacy datenschutz opt out local lokal"],
  ["playback", 1, "sp.taste_meta", "genre mood stimmung tags metadata enrichment"],
  ["playback", 1, "sp.hidden", "hidden ausgeblendet blocked artists künstler"],
  ["playback", 2, "settings.sources.fallback", "youtube soundcloud embed fallback"],
  ["playback", 2, "ytdlp.title", "yt-dlp ytdlp youtube"],
  ["playback", 2, "settings.priority", "priority priorität order reihenfolge audius jamendo archive"],
  ["playback", 3, "sp.player.lyrics", "lyrics songtext liedtext now playing hide ausblenden"],
  ["playback", 3, "sp.player.np_download", "download herunterladen now playing button"],
  ["playback", 3, "sp.player.keys", "hotkey shortcut tastenkürzel taste ändern change rebind belegen zen download"],
  ["audio", 0, "sp.eq.bass", "bass boost tiefen low"],
  ["audio", 0, "sp.eq.headroom", "headroom preamp clipping übersteuern verzerrung distortion pegel gain"],
  ["appearance", 0, "sp.appearance.shape", "visualizer visualisierer hintergrund background shape form glow standard default favorite liebling"],
  ["appearance", 0, "sp.appearance.text", "text size schrift größe ui scale skalierung zoom font"],
  ["storage", 0, "sp.storage.dl_dir", "download folder ordner pfad path speicherort location"],
  ["storage", 0, "sp.storage.reveal", "download folder ordner öffnen show reveal explorer finder"],
  ["storage", 0, "sp.storage.local", "local lokal files dateien folder ordner mp3 flac music musik bibliothek library"],
  ["audio", 0, "sp.eq.enable", "equalizer eq"],
  ["audio", 0, "sp.eq.preset", "preset bass treble höhen tiefen"],
  ["audio", 0, "sp.eq.share", "share teilen export import code datei file"],
  ["appearance", 0, "settings.language", "language sprache"],
  ["appearance", 0, "settings.visuals.music_reactivity", "music musik reaction reaktion beat bass"],
  ["appearance", 0, "settings.visuals.intensity", "background hintergrund glow intensity intensität"],
  ["appearance", 0, "settings.visuals.ascii_covers", "ascii cover"],
  ["appearance", 0, "sp.appearance.startup", "startup start animation splash"],
  ["appearance", 0, "sp.appearance.mini", "mini player miniplayer card karte bar leiste"],
  ["appearance", 0, "sp.appearance.theme", "theme farbe color colour accent akzent farbschema orange blau blue green grün"],
  ["appearance", 0, "sp.appearance.ascii_fx", "ascii animation reveal einblenden skeleton laden loading shimmer"],
  ["appearance", 0, "sp.appearance.throttle", "fps frame rate bildrate background hintergrund akku battery power energie"],
  ["account", 0, "account.handle", "handle name rename umbenennen"],
  ["account", 0, "account.library", "sync api keys schlüssel encryption verschlüsselung e2e library bibliothek"],
  ["account", 0, "account.glyph_links", "share teilen link copy kopieren glyph.my yt-dlp browser"],
  ["account", 1, "profile.avatar", "profile profil picture bild avatar pfp foto photo"],
  ["account", 1, "profile.banner", "profile profil banner header titelbild"],
  ["account", 1, "profile.public", "privacy privatsphäre public öffentlich private privat profile profil"],
  ["account", 1, "profile.show_recent", "privacy privatsphäre recent zuletzt songs history verlauf"],
  ["account", 2, "account.pair", "pair koppeln code device gerät add hinzufügen"],
  ["account", 3, "account.password.change", "password passwort"],
  ["account", 3, "account.passkey.add", "passkey"],
  ["account", 3, "account.recovery", "recovery wiederherstellung code"],
  ["account", 3, "account.delete", "delete löschen account"],
  ["server", 0, "settings.server.url", "server url adresse"],
  ["server", 1, "settings.keys.lastfm", "api key lastfm last.fm"],
  ["server", 1, "settings.keys.youtube", "api key youtube"],
  ["server", 1, "settings.keys.spotify_client_id", "api key spotify"],
  ["server", 1, "settings.keys.jamendo_client_id", "api key jamendo"],
  ["integrations", 0, "sp.discord", "discord rich presence integration integrations status activity listening"],
  ["integrations", 0, "sp.discord.id", "discord application id client id app developer"],
  ["integrations", 1, "sp.mcp", "mcp model context protocol ai claude assistant ki agent"],
  ["integrations", 1, "sp.mcp.access.playback", "mcp ai assistant permissions berechtigungen rechte scopes tools read write lesen schreiben"],
  ["integrations", 1, "sp.mcp.access.settings", "mcp ai assistant settings einstellungen appearance theme equalizer audio playback sync"],
  ["integrations", 1, "sp.mcp.sessions", "mcp ai session sessions sitzungen revoke widerrufen rename umbenennen token connect verbinden"],
  ["plugins", 1, "plugins.catalog", "plugins plugin erweiterungen extensions addons add-ons themes farbschema github topic community"],
  ["plugins", 0, "plugins.publish", "plugin erstellen create publish veröffentlichen developer entwickler github"],
  ["storage", 0, "sp.storage.limit", "cache limit size größe"],
  ["storage", 0, "sp.storage.saver", "data saver datensparmodus mobile traffic daten sparen"],
  ["storage", 0, "sp.storage.usage", "clear leeren cache downloads covers lyrics speicher"],
  ["transfer", 0, "sp.transfer.code", "share teilen export code copy kopieren"],
  ["transfer", 0, "sp.transfer.file", "export file datei save speichern backup"],
  ["transfer", 0, "sp.transfer.import", "import apply übernehmen paste einfügen"],
  ["migrate", 0, "migrate.link", "link playlist import einfügen paste spotify soundcloud deezer apple bandcamp"],
  ["migrate", 0, "migrate.file", "file datei csv m3u xspf json export"],
  ["system", 0, "sp.system.autostart", "autostart start startup boot login anmelden hochfahren pc computer windows autorun"],
  ["system", 0, "sp.system.clipboard_links", "clipboard zwischenablage copy kopieren link spotify youtube paste einfügen"],
  ["system", 0, "sp.system.start_mode","tray systray infobereich taskleiste hintergrund background minimized minimiert hidden versteckt window fenster"],
  ["about", 0, "update.auto", "auto update automatisch"],
  ["about", 0, "update.channel", "prerelease beta channel kanal"],
  ["about", 1, "sp.support.issue", "bug issue report melden"],
  ["about", 1, "sp.support.diag", "diagnostics diagnose"],
  ["about", 1, "sp.support.repo", "github repo repository source quellcode"],
  ["about", 1, "sp.support.data", "data daten folder ordner"],
  ["about", 2, "sp.keys.play", "keyboard shortcuts tastenkürzel tastatur hotkeys keys tasten ändern change rebind belegen"],
  ["about", 2, "sp.keys.zen", "zen cover vollbild fullscreen focus hotkey"],
  ["about", 2, "sp.keys.download", "download herunterladen hotkey"],
  ["about", 2, "sp.player.wheel_seek", "mouse maus wheel mausrad scroll scrollen seek spulen vorspulen progress fortschritt"],
  ["about", 2, "sp.player.wheel_volume", "mouse maus wheel mausrad scroll scrollen volume lautstärke"],
];
