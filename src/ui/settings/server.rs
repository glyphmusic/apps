use super::*;

pub(super) fn server(app: &mut Glyph, cx: &mut Context<Glyph>) -> Div {
    let mode: String = match &app.backend_state {
        BackendState::Connecting => format!("{} {}", app.spinner(), t("status.connecting")),
        BackendState::Ready(BackendMode::Server { url, server_version }) => {
            // The public server is just "Glyph server"; your own shows its host.
            let name = if url.trim_end_matches('/') == glyph_core::config::DEFAULT_SERVER_URL {
                t("settings.server.public").to_string()
            } else {
                url.split("://").nth(1).unwrap_or(url).trim_end_matches('/').to_string()
            };
            tf("settings.server.connected", &[("name", &name), ("v", server_version)]).to_string()
        }
        BackendState::Ready(BackendMode::Direct) => t("status.direct").to_string(),
        BackendState::Ready(BackendMode::Incompatible { protocol_version, .. }) => tf("status.incompatible", &[("v", protocol_version)]).to_string(),
        BackendState::Offline(e) => format!("{} {e}", t("status.backend_offline")),
    };
    let conn = match &app.conn_test {
        None => None,
        Some(ConnTest::Running) => Some((format!("{} {}", app.spinner(), t("settings.server.testing")), white(0.6))),
        Some(ConnTest::Ok(s)) => Some((s.clone(), theme::ok())),
        Some(ConnTest::Failed(s)) => Some((s.clone(), theme::warn())),
    };
    let mut items = vec![
        row(t("settings.server.url"), Some(t("sp.server.url.desc")), input_box(app.server_input.clone())),
        row(
            mode,
            conn.as_ref().map(|(s, _)| SharedString::from(s.clone())),
            div()
                .flex()
                .gap(px(8.0))
                .child(button("conn-test", t("settings.server.test")).on_click(cx.listener(|this, _, _, cx| this.test_connection(cx))))
                .child(button("conn-apply", t("settings.server.apply")).on_click(cx.listener(|this, _, _, cx| this.apply_server(cx)))),
        ),
    ];
    if app.server_auth_required { items.insert(1, row(t("settings.server.bearer"), Some(t("settings.server.bearer.required")), input_box(app.server_bearer_input.clone()))); }
    let keys: Vec<AnyElement> = app
        .key_inputs
        .iter()
        .map(|(name, input)| row(t(&format!("settings.keys.{name}")), None, input_box(input.clone())))
        .collect();
    if app.settings_sub == 1 {
        return div()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .child(section_label(t("settings.keys")))
                    .child(div().pt(px(16.0)).child(info_tip("info-keys", t("settings.keys.hint")))),
            )
            .child(rows(keys));
    }
    rows(items)
}

/// Integrations: services on this computer that Glyph can talk to.
pub(super) fn integrations(app: &mut Glyph, cx: &mut Context<Glyph>) -> Div {
    // With a built-in app ID the field can stay empty; say so instead of
    // asking for an ID the user does not have.
    let custom = !app.config.integrations.discord_client_id.trim().is_empty();
    let id_desc = if !custom && crate::discord::client_id(&app.config).is_some() {
        t("sp.discord.id.builtin")
    } else {
        t("sp.discord.id.desc")
    };
    rows(vec![
        row(
            t("sp.discord"),
            Some(t("sp.discord.desc")),
            switch("discord-presence", app.config.integrations.discord_presence).on_click(
                cx.listener(|this, _, _, cx| {
                    this.set_discord_presence(!this.config.integrations.discord_presence, cx);
                }),
            ),
        ),
        row(t("sp.discord.id"), Some(id_desc), input_box(app.discord_id_input.clone())),
    ])
    .child(mcp_section(app, cx))
}

/// Settings > Integrations: MCP for AI assistants (local or through the server).
pub(super) fn mcp_section(app: &mut Glyph, cx: &mut Context<Glyph>) -> Div {
    use glyph_core::config::McpMode;
    let cfg = app.config.mcp.clone();
    let signed_in = app.signed_in().is_some();
    let mut items = vec![row(
        t("sp.mcp"),
        Some(t("sp.mcp.desc")),
        switch("mcp-on", cfg.enabled).on_click(cx.listener(|this, _, _, cx| this.set_mcp_enabled(!this.config.mcp.enabled, cx))),
    )];
    if cfg.enabled {
        let mode = segmented(
            vec![
                (t("sp.mcp.local"), cfg.mode == McpMode::Local, Rc::new(|this: &mut Glyph, cx: &mut Context<Glyph>| this.set_mcp_mode(McpMode::Local, cx)) as Rc<dyn Fn(&mut Glyph, &mut Context<Glyph>)>),
                (t("sp.mcp.server"), cfg.mode == McpMode::Server, Rc::new(|this: &mut Glyph, cx: &mut Context<Glyph>| this.set_mcp_mode(McpMode::Server, cx))),
            ],
            cx,
            "mcp-mode",
        );
        let mode_desc = match cfg.mode {
            McpMode::Local => t("sp.mcp.local.desc"),
            McpMode::Server if !signed_in => t("sp.mcp.server.signin"),
            McpMode::Server => t("sp.mcp.server.desc"),
        };
        items.push(row(t("sp.mcp.mode"), Some(mode_desc), mode));
        // What the assistant may do, per area: none, read, or read and change.
        {
            use glyph_protocol::mcp::{Access, Area};
            for (area, label, desc, id) in [
                (Area::Playback, "sp.mcp.access.playback", "sp.mcp.access.playback.desc", "mcp-access-playback"),
                (Area::Library, "sp.mcp.access.library", "sp.mcp.access.library.desc", "mcp-access-library"),
                (Area::Stats, "sp.mcp.access.stats", "sp.mcp.access.stats.desc", "mcp-access-stats"),
            ] {
                let current = cfg.access.get(area);
                let mut choices = vec![(Access::None, t("sp.mcp.access.none")), (Access::Read, t("sp.mcp.access.read"))];
                if area.has_write() {
                    choices.push((Access::Write, t("sp.mcp.access.write")));
                }
                let control = segmented(
                    choices
                        .into_iter()
                        .map(|(access, text)| {
                            let pick: Rc<dyn Fn(&mut Glyph, &mut Context<Glyph>)> = Rc::new(move |this: &mut Glyph, cx: &mut Context<Glyph>| this.set_mcp_access(area, access, cx));
                            (text, current == access, pick)
                        })
                        .collect(),
                    cx,
                    id,
                );
                items.push(row(t(label), Some(t(desc)), control));
            }
        }
        let (url, token) = match cfg.mode {
            McpMode::Local => (Some(format!("http://127.0.0.1:{}/mcp", cfg.port)), Some(cfg.token.clone()).filter(|t| !t.is_empty())),
            // OAuth: the assistant only needs the address; it opens the
            // server's sign-in page and the user confirms in this app. A
            // token is the fallback for clients without OAuth.
            McpMode::Server if signed_in => (
                Some(app.mcp.server_token.as_ref().map(|tk| tk.url.clone()).unwrap_or_else(|| format!("{}/mcp", app.config.effective_server_url().trim_end_matches('/')))),
                app.mcp.server_token.as_ref().map(|tk| tk.token.clone()),
            ),
            McpMode::Server => (None, None),
        };
        if let Some(url) = url.clone() {
            let u = url.clone();
            items.push(row(
                t("sp.mcp.url"),
                Some(url.clone().into()),
                button("mcp-copy-url", t("sp.mcp.copy")).on_click(cx.listener(move |this, _, _, cx| {
                    cx.write_to_clipboard(gpui::ClipboardItem::new_string(u.clone()));
                    this.toast(t("sp.mcp.copied"));
                    cx.notify();
                })),
            ));
        }
        match (url, token) {
            (Some(url), Some(token)) => {
                let (tk, cfg_text) = (token.clone(), crate::mcp::client_config(&url, &token));
                let local = cfg.mode == McpMode::Local;
                items.push(row(
                    t("sp.mcp.token"),
                    Some(t("sp.mcp.token.desc")),
                    div()
                        .flex()
                        .gap(px(8.0))
                        .child(button("mcp-copy-token", t("sp.mcp.copy_token")).on_click(cx.listener(move |this, _, _, cx| {
                            cx.write_to_clipboard(gpui::ClipboardItem::new_string(tk.clone()));
                            this.toast(t("sp.mcp.copied"));
                            cx.notify();
                        })))
                        .child(button("mcp-copy-config", t("sp.mcp.copy_config")).on_click(cx.listener(move |this, _, _, cx| {
                            cx.write_to_clipboard(gpui::ClipboardItem::new_string(cfg_text.clone()));
                            this.toast(t("sp.mcp.copied"));
                            cx.notify();
                        })))
                        .child(button("mcp-rotate", t("sp.mcp.rotate")).on_click(cx.listener(move |this, _, _, cx| {
                            if local { this.rotate_mcp_token(cx) } else { this.request_mcp_server_token(cx) }
                        }))),
                ));
            }
            _ if cfg.mode == McpMode::Server && signed_in => {
                let label = if app.mcp.token_loading { t("sp.mcp.creating") } else { t("sp.mcp.create_token") };
                items.push(row(
                    t("sp.mcp.token"),
                    Some(t("sp.mcp.token.server_desc")),
                    button("mcp-create", label).on_click(cx.listener(|this, _, _, cx| this.request_mcp_server_token(cx))),
                ));
            }
            _ => {}
        }
        let status: SharedString = match (&app.mcp.error, cfg.mode) {
            (Some(e), _) => format!("{} {e}", t("sp.mcp.problem")).into(),
            (None, McpMode::Server) if app.mcp.relay_running => t("sp.mcp.relay_on"),
            (None, McpMode::Local) => t("sp.mcp.running"),
            _ => t("sp.mcp.idle"),
        };
        items.push(div().px(px(2.0)).py(px(8.0)).text_size(px(12.5)).text_color(if app.mcp.error.is_some() { theme::warn() } else { white(0.55) }).child(status).into_any_element());
    }
    div().flex().flex_col().child(section_label(t("sp.mcp.section"))).child(rows(items))
}
