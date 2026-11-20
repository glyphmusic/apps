package app.glyph;

/**
 * "Share" and "Open with" Glyph from another app: a see-through window over
 * that app with only the "Add to Glyph" sheet (Liked Songs, playlists, save
 * a shared playlist, play), instead of jumping into the app. It opens at
 * once and reads the link or files while it shows; Done, closing or Back
 * end it and the other app is where it was. What only the app can do (a
 * jam, several links, a search) opens the app.
 *
 * Everything else is {@link MainActivity}: the same page (with `#share`,
 * see glyph-ui share-sheet.svelte.ts) and the same Rust core, which sends
 * its events to every open page (crates/glyph-android: SHEETS).
 */
public class ShareActivity extends MainActivity {
    @Override
    protected boolean isSheet() {
        return true;
    }
}
