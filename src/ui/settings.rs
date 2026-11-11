//! Settings page: the sections listed on the left in groups, the open one on the right as cards
//! of rows (what a setting is on the left, its control on the right). A search over every
//! section shows the settings that match from all of them at once.

use std::sync::Arc;

use egui::{
    pos2, vec2, Align, Color32, CursorIcon, Galley, Id, Layout, Rect, RichText, Sense, Stroke, Ui, UiBuilder, Vec2,
};
use egui_phosphor::regular as icon;

use super::picker::{self, Pick};
use super::theme::{self, *};
use super::widgets;
use super::{Action, Cx};
use crate::config::{expand_home, Config, Paths, SPOTIFY_DEFAULT_CLIENT_ID};
use crate::model::Source;
use crate::service::{AccountStatus, Command};

#[derive(Default)]
pub struct SettingsState {
    new_folder: String,
    import_path: String,
    /// Settings export: where to, and what to leave out.
    export_to: String,
    /// Off until ticked: an exported file is often sent to someone or synced to the cloud.
    export_keys: bool,
    export_no_playlists: bool,
    /// The settings file to import (the newest one found, to begin with).
    settings_file: String,
    settings_file_looked: bool,
    show_spotify_advanced: bool,
    devices_requested: bool,
    /// The section shown on the right.
    open: Sec,
    /// Search settings: while not empty, what matches in every section.
    query: String,
    /// Ctrl+F on this page: the search field takes the keyboard.
    pub focus_search: bool,
}

// ---------------------------------------------------------------------------------- sections

/// One page of settings, listed on the left.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
enum Sec {
    #[default]
    Library,
    Spotify,
    SoundCloud,
    Lastfm,
    Discord,
    Lyrics,
    Playback,
    Audio,
    Downloads,
    Appearance,
    Server,
    Updates,
    Backup,
}

/// A section's name, icon, what it's for (under its title) and other words it's found by.
struct Info {
    label: &'static str,
    icon: &'static str,
    description: &'static str,
    keywords: &'static str,
}

impl Sec {
    fn info(self) -> Info {
        let (label, icon, description, keywords) = match self {
            Sec::Library => (
                "Music folders",
                icon::FOLDER,
                "The folders Utako plays your own music from, and how they're scanned",
                "local library files folder scan rescan add browse drop",
            ),
            Sec::Spotify => (
                "Spotify",
                icon::SPOTIFY_LOGO,
                "Your Spotify account: playback (with Premium), playlists and likes",
                "librespot premium login log in playlists likes quality bitrate normalize cache web api client secret redirect",
            ),
            Sec::SoundCloud => (
                "SoundCloud",
                icon::SOUNDCLOUD_LOGO,
                "Your SoundCloud likes and playlists",
                "oauth token profile url sync likes",
            ),
            Sec::Lastfm => (
                "Last.fm",
                icon::LASTFM_LOGO,
                "Scrobble what you listen to, and see your stats",
                "lastfm scrobbling scrobble api key secret stats",
            ),
            Sec::Discord => (
                "Discord",
                icon::DISCORD_LOGO,
                "Show what you're listening to on Discord",
                "rich presence status activity vesktop arrpc",
            ),
            Sec::Lyrics => (
                "Lyrics",
                icon::MICROPHONE_STAGE,
                "Lyrics from your files, or found online",
                "lrclib genius synced karaoke",
            ),
            Sec::Playback => (
                "Playback",
                icon::PLAY_CIRCLE,
                "Gapless, crossfade, loudness and music videos",
                "crossfade gapless replaygain normalize loudness volume video youtube",
            ),
            Sec::Audio => (
                "Audio output",
                icon::SPEAKER_HIFI,
                "The player and the device your music plays on",
                "mpv output device dac bit perfect exclusive wasapi alsa pipewire sound",
            ),
            Sec::Downloads => (
                "Downloads",
                icon::DOWNLOAD_SIMPLE,
                "Where downloads go, and how Spotify songs and music videos are saved",
                "youtube music videos spotify mp3 ogg vorbis folder lyrics embed tags download media server",
            ),
            Sec::Appearance => (
                "Appearance",
                icon::PALETTE,
                "Themes, colours, the interface's size and covers",
                "theme colour color accent dark holiday scale zoom size covers memory notification",
            ),
            Sec::Server => (
                "Media server",
                icon::HARD_DRIVES,
                "Your mediaserver: its music here, and your settings on your other devices",
                "mediaserver raspberry pi upload sync settings logins devices phone iphone",
            ),
            Sec::Updates => (
                "Updates & about",
                icon::ARROW_CIRCLE_UP,
                "Utako's version and updates, where it keeps its files, and shortcuts",
                "version beta update check install about memory config data shortcuts keys",
            ),
            Sec::Backup => (
                "Backup & move",
                icon::ARCHIVE,
                "Save your settings to a file, or bring them in from another computer",
                "export import backup restore move json keys logins playlists",
            ),
        };
        Info {
            label,
            icon,
            description,
            keywords,
        }
    }

    /// Discord isn't on iOS.
    fn shown(self) -> bool {
        !(cfg!(target_os = "ios") && self == Sec::Discord)
    }
}

/// The sections in groups, each group under a small heading.
const GROUPS: &[(&str, &[Sec])] = &[
    ("Library", &[Sec::Library]),
    (
        "Services",
        &[Sec::Spotify, Sec::SoundCloud, Sec::Lastfm, Sec::Discord, Sec::Lyrics],
    ),
    ("Playback", &[Sec::Playback, Sec::Audio, Sec::Downloads]),
    ("Look", &[Sec::Appearance]),
    ("Devices", &[Sec::Server]),
    ("App", &[Sec::Updates, Sec::Backup]),
];

fn sections() -> impl Iterator<Item = Sec> {
    GROUPS
        .iter()
        .flat_map(|(_, items)| items.iter().copied())
        .filter(|s| s.shown())
}

// ---------------------------------------------------------------------------------- search

/// Lower case, split into words.
fn words(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_string)
        .collect()
}

/// Every word searched for starts a word of the texts ("port" finds Port, not import).
fn matches(search: &[String], texts: &[&str]) -> bool {
    let found: Vec<String> = texts.iter().flat_map(|t| words(t)).collect();
    search.iter().all(|w| found.iter().any(|t| t.starts_with(w.as_str())))
}

/// Whether something shown in the last pass (a group's title, a section's heading) shows. While
/// searching, they only show over settings that matched, which are only known once drawn: the
/// pass is drawn again when that changes.
fn remembered(ui: &Ui, key: Id) -> bool {
    ui.data(|d| d.get_temp::<bool>(key)).unwrap_or(false)
}

fn remember(ui: &Ui, key: Id, was: bool, now: bool) {
    if was != now {
        ui.data_mut(|d| d.insert_temp(key, now));
        ui.ctx().request_discard("settings search");
    }
}

// ---------------------------------------------------------------------------------- layout

/// Inside each row of a card.
const ROW_PAD: egui::Margin = egui::Margin::symmetric(16, 12);
/// Between a row's text and its control.
const ROW_GAP: f32 = 24.0;
/// Narrower text than this puts the control under it.
const MIN_TEXT: f32 = 170.0;
/// Text fields in rows.
const FIELD_W: f32 = 220.0;
/// The list of sections.
const NAV_W: f32 = 196.0;
/// The page shows the list next to the settings from this width; narrower, a menu above them.
const WIDE: f32 = 700.0;

/// The section being drawn, and while searching what's looked for.
struct Page<'a> {
    sec: Sec,
    words: &'a [String],
    /// The section or group matched as a whole: all of it shows.
    all: bool,
    /// Settings shown so far.
    hits: usize,
    /// Rows in the card being drawn (a line goes between them).
    in_card: usize,
    /// The page's spacing between things, rows use it inside.
    gap: f32,
    accent: Color32,
    query: &'a str,
}

impl Page<'_> {
    fn searching(&self) -> bool {
        !self.words.is_empty()
    }

    fn shows(&self, texts: &[&str]) -> bool {
        !self.searching() || self.all || matches(self.words, texts)
    }

    /// A row starts: the line above it, unless it's the card's first.
    fn begin(&mut self, ui: &Ui) {
        if self.in_card > 0 {
            let y = ui.cursor().top();
            ui.painter().hline(ui.max_rect().x_range(), y, Stroke::new(1.0, line()));
        }
        self.in_card += 1;
        self.hits += 1;
    }
}

fn line() -> Color32 {
    with_alpha(Color32::WHITE, 14)
}

/// What a card has on top: nothing, a title (with a line about it) above it, or a title inside
/// that opens it.
#[derive(Clone, Copy)]
enum Head<'a> {
    None,
    Title(&'a str, &'a str),
    Fold(&'a str),
}

/// A card of rows. While searching it shows only with rows that match (all of them when its
/// title does).
fn group(ui: &mut Ui, pg: &mut Page, head: Head, add: impl FnOnce(&mut Ui, &mut Page)) {
    let outer_all = pg.all;
    let (title, help) = match head {
        Head::None => ("", ""),
        Head::Title(t, h) => (t, h),
        Head::Fold(t) => (t, ""),
    };
    if pg.searching() && !title.is_empty() && matches(pg.words, &[title, help]) {
        pg.all = true;
    }
    let key = Id::new(("settings-group", pg.sec, title, pg.query));
    let was = remembered(ui, key);
    let titled = !pg.searching() || pg.all || was;
    if let (Head::Title(t, h), true) = (head, titled) {
        ui.label(RichText::new(t).font(theme::bold_font(15.0)));
        if !h.is_empty() {
            ui.add_space(-2.0);
            ui.label(RichText::new(h).size(13.0).color(pal().text_dim));
            ui.add_space(4.0);
        }
        ui.add_space(4.0);
    }
    let hits_before = pg.hits;
    let bg = ui.painter().add(egui::Shape::Noop);
    let mut header = false;
    // Drawn in a child, the page only makes room for it when it shows.
    let mut child = ui.new_child(
        UiBuilder::new()
            .max_rect(ui.available_rect_before_wrap())
            .layout(Layout::top_down(Align::Min)),
    );
    {
        let ui = &mut child;
        'body: {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 0.0;
            // Fields and buttons stand out from the card (they're the card's colour elsewhere).
            let v = ui.visuals_mut();
            v.extreme_bg_color = pal().panel;
            v.widgets.inactive.weak_bg_fill = pal().selected;
            pg.in_card = 0;
            if let Head::Fold(t) = head {
                let open_id = Id::new(("settings-fold", pg.sec, t));
                let mut open = ui.data(|d| d.get_temp::<bool>(open_id)).unwrap_or(false);
                if titled {
                    header = true;
                    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 46.0), Sense::click());
                    let resp = resp.on_hover_cursor(CursorIcon::PointingHand);
                    if resp.clicked() && !pg.searching() {
                        open = !open;
                        ui.data_mut(|d| d.insert_temp(open_id, open));
                    }
                    let color = if resp.hovered() { TEXT } else { pal().text_dim };
                    ui.painter().text(
                        pos2(rect.left() + 16.0, rect.center().y),
                        egui::Align2::LEFT_CENTER,
                        t,
                        theme::font(14.5),
                        TEXT,
                    );
                    let caret = if open || pg.searching() {
                        icon::CARET_UP
                    } else {
                        icon::CARET_DOWN
                    };
                    theme::paint_icon(
                        ui.painter(),
                        pos2(rect.right() - 24.0, rect.center().y),
                        caret,
                        icon_font(15.0),
                        color,
                    );
                    pg.in_card = 1;
                }
                if !open && !pg.searching() {
                    break 'body;
                }
            }
            add(ui, pg);
        }
    }
    let rect = child.min_rect();
    let found = pg.hits > hits_before;
    if found || header {
        ui.advance_cursor_after_rect(rect);
        ui.painter().set(
            bg,
            egui::epaint::RectShape::new(
                rect,
                RADIUS,
                pal().card,
                Stroke::new(1.0, with_alpha(Color32::WHITE, 8)),
                egui::StrokeKind::Inside,
            ),
        );
    }
    if pg.searching() && !title.is_empty() && !pg.all {
        remember(ui, key, was, found);
    }
    if found || header || (titled && !title.is_empty()) {
        ui.add_space(18.0);
    }
    pg.all = outer_all;
}

/// Lines of text, wrapped at `width`, and how tall they are together.
fn text_block(ui: &Ui, lines: &[(&str, f32, Color32)], width: f32) -> (Vec<Arc<Galley>>, f32) {
    let mut out: Vec<Arc<Galley>> = Vec::new();
    let mut height = 0.0;
    for (text, size, color) in lines {
        if text.is_empty() {
            continue;
        }
        let galley = ui
            .painter()
            .layout(text.to_string(), theme::font(*size), *color, width.max(1.0));
        if !out.is_empty() {
            height += 3.0;
        }
        height += galley.size().y;
        out.push(galley);
    }
    (out, height)
}

fn paint_block(ui: &Ui, galleys: &[Arc<Galley>], top_left: egui::Pos2) {
    let mut y = top_left.y;
    for g in galleys {
        ui.painter().galley(pos2(top_left.x, y), g.clone(), TEXT);
        y += g.size().y + 3.0;
    }
}

/// A setting: what it is on the left (`status` under the name, in its colour, then `help`),
/// its control on the right. When the two don't fit side by side, the control goes underneath.
fn row_with(
    ui: &mut Ui,
    pg: &mut Page,
    label: &str,
    status: Option<(&str, Color32)>,
    help: &str,
    keywords: &str,
    control: impl FnOnce(&mut Ui),
) {
    if !pg.shows(&[label, help, keywords]) {
        return;
    }
    pg.begin(ui);
    // The control's size, from the last pass: the text wraps beside it.
    let id = Id::new(("settings-row", pg.sec, label));
    let known: Vec2 = ui.data(|d| d.get_temp(id)).unwrap_or(Vec2::ZERO);
    let gap = pg.gap;
    egui::Frame::new().inner_margin(ROW_PAD).show(ui, |ui| {
        let w = ui.available_width();
        ui.set_width(w);
        ui.spacing_mut().item_spacing.y = gap;
        let side = w - known.x - ROW_GAP >= MIN_TEXT;
        let text_w = if side && known.x > 0.0 {
            w - known.x - ROW_GAP
        } else {
            w
        };
        let (status_text, status_color) = status.unwrap_or(("", TEXT));
        let (galleys, text_h) = text_block(
            ui,
            &[
                (label, 14.0, TEXT),
                (status_text, 12.5, status_color),
                (help, 12.5, pal().text_dim),
            ],
            text_w,
        );
        let used = if side {
            let h = text_h.max(known.y);
            let (rect, _) = ui.allocate_exact_size(vec2(w, h), Sense::hover());
            paint_block(ui, &galleys, pos2(rect.left(), rect.center().y - text_h / 2.0));
            // Its width known, the control ends at the right edge; the first time it's measured.
            let ctrl_w = if known.x > 0.0 {
                known.x
            } else {
                (w - MIN_TEXT).max(0.0)
            };
            let area = Rect::from_min_max(pos2(rect.right() - ctrl_w, rect.top()), rect.max);
            let mut child = ui.new_child(
                UiBuilder::new()
                    .max_rect(area)
                    .layout(Layout::left_to_right(Align::Center)),
            );
            control(&mut child);
            child.min_rect().size()
        } else {
            let (rect, _) = ui.allocate_exact_size(vec2(w, text_h), Sense::hover());
            paint_block(ui, &galleys, rect.min);
            ui.add_space(8.0);
            ui.horizontal_wrapped(|ui| control(ui)).response.rect.size()
        };
        if (used - known).abs().max_elem() > 0.5 {
            ui.data_mut(|d| d.insert_temp(id, used));
            ui.ctx().request_discard("settings row");
        }
    });
}

fn row(ui: &mut Ui, pg: &mut Page, label: &str, help: &str, keywords: &str, control: impl FnOnce(&mut Ui)) {
    row_with(ui, pg, label, None, help, keywords, control);
}

/// A setting that's on or off, with a switch.
fn toggle(ui: &mut Ui, pg: &mut Page, label: &str, help: &str, value: &mut bool) -> bool {
    let accent = pg.accent;
    let mut changed = false;
    row(ui, pg, label, help, "", |ui| {
        changed = switch(ui, value, accent).changed()
    });
    changed
}

/// A text field beside its name.
fn field(ui: &mut Ui, pg: &mut Page, label: &str, help: &str, value: &mut String, hint_text: &str, password: bool) {
    row(ui, pg, label, help, "", |ui| {
        ui.add(
            egui::TextEdit::singleline(value)
                .hint_text(hint_text)
                .password(password)
                .desired_width(FIELD_W),
        );
    });
}

/// A setting whose controls take the card's width, under its name; `help` goes below them.
fn stack(ui: &mut Ui, pg: &mut Page, label: &str, help: &str, keywords: &str, add: impl FnOnce(&mut Ui)) {
    if !pg.shows(&[label, help, keywords]) {
        return;
    }
    pg.begin(ui);
    let gap = pg.gap;
    egui::Frame::new().inner_margin(ROW_PAD).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.spacing_mut().item_spacing.y = gap;
        if !label.is_empty() {
            ui.label(RichText::new(label).size(14.0).color(TEXT));
            ui.add_space(2.0);
        }
        add(ui);
        if !help.is_empty() {
            ui.label(RichText::new(help).size(12.5).color(pal().text_dim));
        }
    });
}

/// A line of text in a card (found by `keywords` and the text).
fn note(ui: &mut Ui, pg: &mut Page, text: &str, keywords: &str) {
    if !pg.shows(&[text, keywords]) {
        return;
    }
    pg.begin(ui);
    egui::Frame::new().inner_margin(ROW_PAD).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.label(RichText::new(text).size(12.5).color(pal().text_dim));
    });
}

/// An on/off switch, in the accent colour when on.
fn switch(ui: &mut Ui, on: &mut bool, accent: Color32) -> egui::Response {
    let (rect, mut resp) = ui.allocate_exact_size(vec2(38.0, 22.0), Sense::click());
    if resp.clicked() {
        *on = !*on;
        resp.mark_changed();
    }
    let value = *on;
    resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Checkbox, ui.is_enabled(), value, ""));
    let t = ui.ctx().animate_bool_responsive(resp.id, value);
    let off = mix(pal().card, TEXT, if resp.hovered() { 0.26 } else { 0.18 });
    let track = mix(off, accent, t);
    let p = ui.painter();
    p.rect_filled(rect, rect.height() / 2.0, track);
    let r = rect.height() / 2.0 - 3.0;
    let x = egui::lerp((rect.left() + r + 3.0)..=(rect.right() - r - 3.0), t);
    p.circle_filled(pos2(x, rect.center().y), r, mix(TEXT, theme::on_color(accent), t));
    resp.on_hover_cursor(CursorIcon::PointingHand)
}

/// What buttons with these labels take in a row, the spacing before each included: the width a
/// text field ahead of them leaves free, so the row never runs past the page.
fn buttons_width(ui: &Ui, labels: impl IntoIterator<Item = egui::WidgetText>) -> f32 {
    let spacing = ui.spacing();
    labels
        .into_iter()
        .map(|label| {
            let text = label.into_galley(ui, Some(egui::TextWrapMode::Extend), f32::INFINITY, egui::TextStyle::Button);
            text.size().x + 2.0 * spacing.button_padding.x + spacing.item_spacing.x
        })
        .sum::<f32>()
        // The field's own margin, outside its desired width.
        + 10.0
}

const GREEN: Color32 = Color32::from_rgb(0x4a, 0xd6, 0x8a);

/// An account's state, in its colour.
fn status(s: &AccountStatus) -> (String, Color32) {
    match s {
        AccountStatus::Off => ("Not connected".to_string(), pal().text_faint),
        AccountStatus::Working(m) => (m.clone(), pal().text_dim),
        AccountStatus::Connected(name) if name.is_empty() => ("Connected".to_string(), GREEN),
        AccountStatus::Connected(name) => (format!("Connected as {name}"), GREEN),
        AccountStatus::Error(e) => (e.clone(), DANGER),
    }
}

// ---------------------------------------------------------------------------------- the page

pub fn show(ui: &mut Ui, cx: &mut Cx, cfg: &mut Config, st: &mut SettingsState, paths: &Paths, rss_mb: f32) {
    take_picks(ui, cx, cfg, st);
    if !st.open.shown() {
        st.open = Sec::default();
    }
    let wide = ui.available_width() >= WIDE;
    egui::Frame::new()
        .inner_margin(egui::Margin {
            left: 24,
            right: 8,
            top: 4,
            bottom: 0,
        })
        .show(ui, |ui| {
            let height = ui.available_height();
            if wide {
                ui.horizontal_top(|ui| {
                    ui.allocate_ui_with_layout(vec2(NAV_W, height), Layout::top_down(Align::Min), |ui| {
                        ui.set_width(NAV_W);
                        ui.set_min_height(height);
                        aside(ui, cx.accent, st);
                    });
                    ui.add_space(28.0);
                    ui.allocate_ui_with_layout(
                        vec2(ui.available_width(), height),
                        Layout::top_down(Align::Min),
                        |ui| {
                            content(ui, cx, cfg, st, paths, rss_mb, true);
                        },
                    );
                });
            } else {
                ui.label(RichText::new("Settings").font(theme::bold_font(26.0)));
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    let menu_w = 210.0_f32.min(ui.available_width() * 0.45);
                    search_field(ui, st, ui.available_width() - menu_w - 24.0);
                    let current = st.open.info();
                    egui::ComboBox::from_id_salt("settings-section")
                        .width(menu_w)
                        .selected_text(theme::ic(current.icon, current.label))
                        .truncate()
                        .show_ui(ui, |ui| {
                            for (group, items) in GROUPS {
                                for &sec in items.iter().filter(|s| s.shown()) {
                                    let info = sec.info();
                                    let text = theme::ic(info.icon, format!("{group} › {}", info.label));
                                    if ui.add(egui::Button::selectable(st.open == sec, text)).clicked() {
                                        st.open = sec;
                                        st.query.clear();
                                    }
                                }
                            }
                        });
                });
                ui.add_space(12.0);
                content(ui, cx, cfg, st, paths, rss_mb, false);
            }
        });
}

/// The picked files and folders, whichever section is open when they come back.
fn take_picks(ui: &Ui, cx: &mut Cx, cfg: &mut Config, st: &mut SettingsState) {
    let ctx = ui.ctx();
    match picker::take(ctx, "library-folder") {
        Some(Ok(p)) if !cfg.library.folders.contains(&p) => cfg.library.folders.push(p),
        Some(Err(e)) => cx.actions.push(Action::Cmd(Command::Notify(e))),
        _ => {}
    }
    match picker::take(ctx, "download-folder") {
        Some(Ok(p)) => cfg.downloads.folder = p.to_string_lossy().into_owned(),
        Some(Err(e)) => cx.actions.push(Action::Cmd(Command::Notify(e))),
        None => {}
    }
    match picker::take(ctx, "settings-file") {
        Some(Ok(p)) => st.settings_file = p.to_string_lossy().into_owned(),
        Some(Err(e)) => cx.actions.push(Action::Cmd(Command::Notify(e))),
        None => {}
    }
    #[cfg(not(target_os = "ios"))]
    for id in ["server-upload-folder", "server-upload-song"] {
        match picker::take(ctx, id) {
            Some(Ok(path)) => cx.actions.push(Action::Cmd(Command::ServerUpload(path))),
            Some(Err(e)) => cx.actions.push(Action::Cmd(Command::Notify(e))),
            None => {}
        }
    }
}

fn search_field(ui: &mut Ui, st: &mut SettingsState, width: f32) {
    let r = ui.add(
        egui::TextEdit::singleline(&mut st.query)
            .id_salt("settings-search")
            .hint_text(theme::ic(icon::MAGNIFYING_GLASS, "Search settings"))
            .margin(egui::Margin::symmetric(10, 7))
            .desired_width(width),
    );
    if std::mem::take(&mut st.focus_search) {
        r.request_focus();
    }
    if (r.has_focus() || r.lost_focus()) && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
        st.query.clear();
    }
}

/// The title, the search field and the sections in their groups.
fn aside(ui: &mut Ui, accent: Color32, st: &mut SettingsState) {
    ui.label(RichText::new("Settings").font(theme::bold_font(28.0)));
    ui.add_space(10.0);
    search_field(ui, st, NAV_W - 20.0);
    ui.add_space(10.0);
    let searching = !words(&st.query).is_empty();
    egui::ScrollArea::vertical()
        .id_salt("settings-nav")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            // Room on the right for the scroll bar.
            ui.set_width(NAV_W - 10.0);
            ui.spacing_mut().item_spacing.y = 2.0;
            for (group, items) in GROUPS {
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.add_space(10.0);
                    ui.label(RichText::new(*group).size(11.5).color(pal().text_faint));
                });
                for &sec in items.iter().filter(|s| s.shown()) {
                    if nav_item(ui, sec, !searching && st.open == sec, accent).clicked() {
                        st.open = sec;
                        st.query.clear();
                    }
                }
                ui.add_space(10.0);
            }
        });
}

fn nav_item(ui: &mut Ui, sec: Sec, on: bool, accent: Color32) -> egui::Response {
    let info = sec.info();
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 32.0), Sense::click());
    let hovered = resp.hovered();
    let fill = if on {
        with_alpha(Color32::WHITE, 18)
    } else if hovered {
        with_alpha(Color32::WHITE, 12)
    } else {
        Color32::TRANSPARENT
    };
    let p = ui.painter();
    p.rect_filled(rect, 8.0, fill);
    let icon_color = if on {
        accent
    } else if hovered {
        TEXT
    } else {
        pal().text_dim
    };
    theme::paint_icon(
        p,
        pos2(rect.left() + 20.0, rect.center().y),
        info.icon,
        icon_font(16.0),
        icon_color,
    );
    let text_color = if on || hovered { TEXT } else { pal().text_dim };
    let font = theme::font(13.5);
    let galley_h = ui.ctx().fonts_mut(|f| f.row_height(&font));
    widgets::text_trunc(
        ui,
        pos2(rect.left() + 38.0, rect.center().y - galley_h / 2.0),
        info.label,
        font,
        text_color,
        rect.width() - 46.0,
    );
    resp.on_hover_cursor(CursorIcon::PointingHand)
}

/// The open section, or what's found in every section.
fn content(ui: &mut Ui, cx: &mut Cx, cfg: &mut Config, st: &mut SettingsState, paths: &Paths, rss_mb: f32, wide: bool) {
    let search = words(&st.query);
    let gap = ui.spacing().item_spacing.y;
    let salt = if search.is_empty() {
        format!("settings-{:?}", st.open)
    } else {
        "settings-search".to_string()
    };
    egui::ScrollArea::vertical()
        .id_salt(salt)
        .auto_shrink([false, false])
        .show(ui, |ui| {
            egui::Frame::new()
                .inner_margin(egui::Margin {
                    left: 0,
                    right: 16,
                    top: 0,
                    bottom: 24,
                })
                .show(ui, |ui| {
                    ui.set_max_width(ui.available_width().min(760.0));
                    ui.set_width(ui.available_width());
                    if wide {
                        // The title lines up with the search field on the left.
                        ui.add_space(50.0);
                    }
                    if search.is_empty() {
                        let info = st.open.info();
                        ui.label(RichText::new(info.label).font(theme::bold_font(21.0)));
                        ui.label(RichText::new(info.description).size(13.5).color(pal().text_dim));
                        ui.add_space(18.0);
                        let mut pg = Page {
                            sec: st.open,
                            words: &search,
                            all: false,
                            hits: 0,
                            in_card: 0,
                            gap,
                            accent: cx.accent,
                            query: "",
                        };
                        body(ui, &mut pg, cx, cfg, st, paths, rss_mb);
                        return;
                    }
                    let query = st.query.trim().to_lowercase();
                    let mut found = 0;
                    let mut open = None;
                    for sec in sections() {
                        let info = sec.info();
                        let all = matches(&search, &[info.label, info.description, info.keywords]);
                        let key = Id::new(("settings-section-found", sec, query.as_str()));
                        let was = remembered(ui, key);
                        if all || was {
                            let r = ui
                                .horizontal(|ui| {
                                    ui.label(
                                        RichText::new(info.icon)
                                            .family(theme::icons())
                                            .size(17.0)
                                            .color(pal().text_dim),
                                    );
                                    let r = ui.add(
                                        egui::Label::new(RichText::new(info.label).font(theme::bold_font(18.0)))
                                            .sense(Sense::click()),
                                    );
                                    ui.label(
                                        RichText::new(icon::CARET_RIGHT)
                                            .family(theme::icons())
                                            .size(14.0)
                                            .color(pal().text_faint),
                                    );
                                    r
                                })
                                .inner
                                .on_hover_cursor(CursorIcon::PointingHand)
                                .on_hover_text("Open this section");
                            if r.clicked() {
                                open = Some(sec);
                            }
                            ui.add_space(8.0);
                        }
                        let mut pg = Page {
                            sec,
                            words: &search,
                            all,
                            hits: 0,
                            in_card: 0,
                            gap,
                            accent: cx.accent,
                            query: &query,
                        };
                        body(ui, &mut pg, cx, cfg, st, paths, rss_mb);
                        if !all {
                            remember(ui, key, was, pg.hits > 0);
                        }
                        if all || was {
                            ui.add_space(16.0);
                        }
                        found += pg.hits;
                    }
                    if found == 0 {
                        ui.add_space(40.0);
                        ui.vertical_centered(|ui| {
                            ui.label(
                                RichText::new(icon::MAGNIFYING_GLASS)
                                    .family(theme::icons())
                                    .size(34.0)
                                    .color(pal().text_faint),
                            );
                            ui.add_space(6.0);
                            ui.label(RichText::new("No settings match").font(theme::bold_font(16.0)));
                            ui.label(RichText::new("Try other words, or pick a section.").color(pal().text_dim));
                        });
                    }
                    if let Some(sec) = open {
                        st.open = sec;
                        st.query.clear();
                    }
                });
        });
}

fn body(ui: &mut Ui, pg: &mut Page, cx: &mut Cx, cfg: &mut Config, st: &mut SettingsState, paths: &Paths, rss_mb: f32) {
    match pg.sec {
        Sec::Library => library(ui, pg, cx, cfg, st),
        Sec::Spotify => spotify(ui, pg, cx, cfg, st),
        Sec::SoundCloud => soundcloud(ui, pg, cx, cfg),
        Sec::Lastfm => lastfm(ui, pg, cx, cfg),
        Sec::Discord => {
            #[cfg(not(target_os = "ios"))]
            discord(ui, pg, cfg);
        }
        Sec::Lyrics => lyrics(ui, pg, cfg),
        Sec::Playback => playback(ui, pg, cfg),
        Sec::Audio => audio(ui, pg, cx, cfg, st),
        Sec::Downloads => downloads(ui, pg, cx, cfg),
        Sec::Appearance => appearance(ui, pg, cx, cfg),
        Sec::Server => media_server(ui, pg, cx, cfg),
        Sec::Updates => updates(ui, pg, cx, cfg, paths, rss_mb),
        Sec::Backup => backup(ui, pg, cx, st),
    }
}

// ---------------------------------------------------------------------------------- library

fn library(ui: &mut Ui, pg: &mut Page, cx: &mut Cx, cfg: &mut Config, st: &mut SettingsState) {
    let ios = cfg!(target_os = "ios");
    group(ui, pg, Head::Title("Folders", ""), |ui, pg| {
        let mut remove = None;
        for (i, f) in cfg.library.folders.iter().enumerate() {
            let path = f.display().to_string();
            row(ui, pg, &path, "", "folder library", |ui| {
                if widgets::icon_button(ui, icon::X, 13.0, pal().text_dim, "Remove folder").clicked() {
                    remove = Some(i);
                }
            });
        }
        if let Some(i) = remove {
            cfg.library.folders.remove(i);
        }
        if cfg.library.folders.is_empty() && !ios {
            note(
                ui,
                pg,
                "No folders yet: add one below, or drop a folder onto the window.",
                "folder library",
            );
        }
        if ios {
            note(
                ui,
                pg,
                "Copy music into Files › On My iPhone › Utako, then Rescan now.",
                "folder library files",
            );
        }
        if !ios {
            stack(
                ui,
                pg,
                "Add a folder",
                "Or drop a folder onto the window.",
                "library browse",
                |ui| {
                    ui.horizontal(|ui| {
                        let r = ui.add(
                            egui::TextEdit::singleline(&mut st.new_folder)
                                .hint_text("~/Music")
                                .desired_width(
                                    ui.available_width()
                                        - buttons_width(ui, ["Add".into(), theme::ic(icon::FOLDER_OPEN, "Browse…")]),
                                ),
                        );
                        let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                        if (ui.button("Add").clicked() || enter) && !st.new_folder.trim().is_empty() {
                            let p = expand_home(&st.new_folder);
                            if !cfg.library.folders.contains(&p) {
                                cfg.library.folders.push(p);
                            }
                            st.new_folder.clear();
                        }
                        if ui.button(theme::ic(icon::FOLDER_OPEN, "Browse…")).clicked() {
                            picker::open(ui.ctx(), "library-folder", "Add a music folder", Pick::Folder);
                        }
                    });
                },
            );
        }
    });
    group(ui, pg, Head::Title("Scanning", ""), |ui, pg| {
        toggle(
            ui,
            pg,
            "Rescan on startup",
            "Only new or changed files are read again, so rescans are quick.",
            &mut cfg.library.scan_on_startup,
        );
        row(
            ui,
            pg,
            "Rescan now",
            "Find new, changed and removed songs in your folders.",
            "scan",
            |ui| {
                if ui.button(theme::ic(icon::ARROWS_CLOCKWISE, "Rescan")).clicked() {
                    cx.actions.push(Action::Cmd(Command::Rescan));
                }
            },
        );
    });
    if ios {
        return;
    }
    group(ui, pg, Head::Title("Playlist files", ""), |ui, pg| {
        stack(
            ui,
            pg,
            "Import an .m3u playlist",
            "Its songs become a playlist. Or drop the file onto the window.",
            "import playlist m3u m3u8",
            |ui| {
                ui.horizontal(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut st.import_path)
                            .hint_text("~/Downloads/Playlist.m3u")
                            .desired_width(ui.available_width() - buttons_width(ui, ["Import".into()])),
                    );
                    if ui.button("Import").clicked() && !st.import_path.trim().is_empty() {
                        let p = expand_home(&st.import_path);
                        cx.actions.push(Action::Cmd(Command::ImportM3u(p)));
                        st.import_path.clear();
                    }
                });
            },
        );
    });
}

// ---------------------------------------------------------------------------------- services

fn spotify(ui: &mut Ui, pg: &mut Page, cx: &mut Cx, cfg: &mut Config, st: &mut SettingsState) {
    group(ui, pg, Head::Title("Account", ""), |ui, pg| {
        let (text, color) = status(&cx.feed.spotify);
        let working = matches!(cx.feed.spotify, AccountStatus::Working(_));
        row_with(
            ui,
            pg,
            "Spotify account",
            Some((&text, color)),
            "Playback needs Premium. A free account can still import playlists.",
            "login log in log out sync playlists",
            |ui| {
                if working {
                    ui.spinner();
                }
                if cx.feed.spotify_logged_in {
                    if ui.button(theme::ic(icon::ARROWS_CLOCKWISE, "Sync playlists")).clicked() {
                        cx.actions.push(Action::Cmd(Command::SyncSpotify));
                    }
                    if ui.button("Log out").clicked() {
                        cx.actions.push(Action::Cmd(Command::SpotifyLogout));
                    }
                } else if widgets::pill(ui, "Log in with Spotify", source_color(Source::Spotify), Color32::BLACK)
                    .clicked()
                {
                    cx.actions.push(Action::Cmd(Command::SpotifyLogin));
                }
            },
        );
        toggle(ui, pg, "Enable Spotify", "", &mut cfg.spotify.enabled);
    });
    group(ui, pg, Head::Title("Sound", ""), |ui, pg| {
        row(
            ui,
            pg,
            "Quality",
            "Spotify sends its lossless streams only to its own apps: 320 kbps Ogg Vorbis at most here.",
            "bitrate kbps lossless flac",
            |ui| {
                egui::ComboBox::from_id_salt("bitrate")
                    .selected_text(format!("{} kbps", cfg.spotify.bitrate))
                    .show_ui(ui, |ui| {
                        for b in [96u16, 160, 320] {
                            ui.selectable_value(&mut cfg.spotify.bitrate, b, format!("{b} kbps"));
                        }
                        ui.add_enabled(false, egui::Button::selectable(false, "Lossless (FLAC)"))
                            .on_disabled_hover_text(
                                "320 kbps Ogg Vorbis is the most Spotify gives players other than \
                                 its own.",
                            );
                    });
            },
        );
        toggle(
            ui,
            pg,
            "Normalize volume",
            "Plays every song about as loud.",
            &mut cfg.spotify.normalisation,
        );
        toggle(
            ui,
            pg,
            "Cache audio on disk",
            "Saves bandwidth, up to 2 GB.",
            &mut cfg.spotify.cache_audio,
        );
        // Windows and macOS have one system output that Spotify follows.
        if cfg!(target_os = "linux") {
            row(
                ui,
                pg,
                "Audio output",
                "Automatic follows your system's output. Changes apply when Spotify next starts \
                 playing.",
                "pulseaudio pipewire alsa",
                |ui| {
                    let label = |v: &str| match v {
                        "pulseaudio" => "PipeWire / PulseAudio",
                        "alsa" => "ALSA",
                        _ => "Automatic",
                    };
                    egui::ComboBox::from_id_salt("spotify-output")
                        .selected_text(label(&cfg.spotify.audio_output))
                        .show_ui(ui, |ui| {
                            for v in ["auto", "pulseaudio", "alsa"] {
                                ui.selectable_value(&mut cfg.spotify.audio_output, v.to_string(), label(v));
                            }
                        });
                },
            );
        }
    });
    group(ui, pg, Head::Fold("Advanced"), |ui, pg| {
        st.show_spotify_advanced = true;
        stack(
            ui,
            pg,
            "Your own Spotify app (recommended for search)",
            "Search and artist pages use Spotify's Web API, whose shared key is often rate \
             limited. Your own free app fixes that: developer.spotify.com → Dashboard → Create \
             app → tick \"Web API\", then paste its Client ID and Client secret here.",
            "web api rate limit 429 developer",
            |_| {},
        );
        field(
            ui,
            pg,
            "Client ID",
            "",
            &mut cfg.spotify.web_api_client_id,
            "e.g. 1a2b3c4d5e6f…",
            false,
        );
        field(
            ui,
            pg,
            "Client secret",
            "With the secret there's no browser login and no Redirect URI.",
            &mut cfg.spotify.web_api_client_secret,
            "Settings → View client secret",
            true,
        );
        let (text, color) = status(&cx.feed.spotify_web_api);
        row_with(ui, pg, "Your app", Some((&text, color)), "", "web api", |_| {});
        stack(
            ui,
            pg,
            "Or log in to your app in the browser",
            "\"redirect_uri: Not matching configuration\": add the Redirect URI above to your app \
             (Settings → Edit → Redirect URIs → Add → Save). Or use the Client secret instead.",
            "redirect uri authorize",
            |ui| {
                ui.label(RichText::new("Redirect URI (must match your app's exactly)").color(pal().text_dim));
                let default_uri = crate::player::oauth::redirect_uri(cfg.spotify.web_api_redirect_port);
                ui.horizontal(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut cfg.spotify.web_api_redirect_uri)
                            .hint_text(&default_uri)
                            .font(egui::TextStyle::Monospace)
                            .desired_width(ui.available_width() - buttons_width(ui, [theme::ic(icon::COPY, "Copy")])),
                    );
                    if ui.small_button(theme::ic(icon::COPY, "Copy")).clicked() {
                        ui.ctx().copy_text(cfg.spotify.web_api_redirect());
                    }
                });
                let redirect_ok = match crate::player::oauth::parse_redirect(&cfg.spotify.web_api_redirect()) {
                    Ok(_) => true,
                    Err(e) => {
                        ui.label(RichText::new(format!("{e}")).size(12.0).color(DANGER));
                        false
                    }
                };
                ui.label(
                    RichText::new(format!("Empty: {default_uri}. Use http:// with 127.0.0.1."))
                        .size(12.5)
                        .color(pal().text_dim),
                );
                let ready = !cfg.spotify.web_api_client_id.trim().is_empty() && redirect_ok;
                if ui
                    .add_enabled(ready, egui::Button::new(theme::ic(icon::SIGN_IN, "Authorize")))
                    .clicked()
                {
                    cx.actions.push(Action::Cmd(Command::SpotifyWebApiLogin));
                }
            },
        );
        let wrong_id = cfg.spotify.client_id.trim() != SPOTIFY_DEFAULT_CLIENT_ID;
        let warning = wrong_id.then_some((
            "Playback only works with Spotify's own ID here. Put your app's ID in the Client ID \
             field above, then click Reset.",
            DANGER,
        ));
        row_with(
            ui,
            pg,
            "Login client ID",
            warning,
            "Leave the login client on Spotify's own ID: playback only works with it.",
            "",
            |ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut cfg.spotify.client_id)
                        .hint_text(SPOTIFY_DEFAULT_CLIENT_ID)
                        .desired_width(FIELD_W),
                );
            },
        );
        row(ui, pg, "Login redirect port", "", "reset", |ui| {
            ui.add(egui::DragValue::new(&mut cfg.spotify.redirect_port).range(1024..=65535));
            if ui.button("Reset").clicked() {
                cfg.spotify.client_id = SPOTIFY_DEFAULT_CLIENT_ID.into();
                cfg.spotify.redirect_port = 8898;
            }
        });
    });
}

fn soundcloud(ui: &mut Ui, pg: &mut Page, cx: &mut Cx, cfg: &mut Config) {
    group(ui, pg, Head::None, |ui, pg| {
        let (text, color) = status(&cx.feed.soundcloud);
        let accent = pg.accent;
        row_with(ui, pg, "Enable SoundCloud", Some((&text, color)), "", "", |ui| {
            if matches!(cx.feed.soundcloud, AccountStatus::Working(_)) {
                ui.spinner();
            }
            switch(ui, &mut cfg.soundcloud.enabled, accent);
        });
        field(
            ui,
            pg,
            "Profile URL",
            "",
            &mut cfg.soundcloud.profile_url,
            "https://soundcloud.com/yourname",
            false,
        );
        field(
            ui,
            pg,
            "OAuth token",
            "For private likes and playlists. On soundcloud.com: DevTools → Application → Cookies \
             → oauth_token.",
            &mut cfg.soundcloud.oauth_token,
            "2-123456-…",
            true,
        );
        row(
            ui,
            pg,
            "Likes and playlists",
            "Import them from SoundCloud now.",
            "sync",
            |ui| {
                if ui.button(theme::ic(icon::ARROWS_CLOCKWISE, "Sync")).clicked() {
                    cx.actions.push(Action::Cmd(Command::SyncSoundCloud));
                }
            },
        );
    });
}

fn lastfm(ui: &mut Ui, pg: &mut Page, cx: &mut Cx, cfg: &mut Config) {
    group(ui, pg, Head::None, |ui, pg| {
        let (text, color) = status(&cx.feed.lastfm);
        let accent = pg.accent;
        row_with(
            ui,
            pg,
            "Scrobble what I listen to",
            Some((&text, color)),
            "",
            "",
            |ui| {
                if matches!(cx.feed.lastfm, AccountStatus::Working(_)) {
                    ui.spinner();
                }
                switch(ui, &mut cfg.lastfm.enabled, accent);
            },
        );
        let help = if cfg.lastfm.scrobble_instantly {
            "Songs scrobble as they start, even if you skip them."
        } else {
            "Songs scrobble after half their length, or 4 minutes."
        };
        toggle(
            ui,
            pg,
            "Scrobble as soon as a song starts",
            help,
            &mut cfg.lastfm.scrobble_instantly,
        );
        field(
            ui,
            pg,
            "API key",
            "From last.fm/api/account/create.",
            &mut cfg.lastfm.api_key,
            "",
            false,
        );
        field(ui, pg, "API secret", "", &mut cfg.lastfm.api_secret, "", true);
        let connected = matches!(cx.feed.lastfm, AccountStatus::Connected(_));
        row(ui, pg, "Your Last.fm account", "", "connect disconnect stats", |ui| {
            if connected {
                if ui.button(theme::ic(icon::CHART_BAR, "Your stats")).clicked() {
                    cx.actions.push(Action::Go(super::View::Profile));
                }
                if ui.button("Disconnect").clicked() {
                    cx.actions.push(Action::Cmd(Command::LastfmLogout));
                }
            } else if ui.button(theme::ic(icon::SIGN_IN, "Connect account")).clicked() {
                cx.actions.push(Action::Cmd(Command::LastfmLogin));
            }
        });
    });
}

#[cfg(not(target_os = "ios"))]
fn discord(ui: &mut Ui, pg: &mut Page, cfg: &mut Config) {
    group(ui, pg, Head::None, |ui, pg| {
        toggle(
            ui,
            pg,
            "Show what I'm listening to on Discord",
            "",
            &mut cfg.discord.enabled,
        );
        field(
            ui,
            pg,
            "Application ID",
            "The Application ID of a free app from discord.com/developers/applications. Works \
             with Discord, Vesktop and arRPC.",
            &mut cfg.discord.app_id,
            "e.g. 1234567890123456789",
            false,
        );
        toggle(
            ui,
            pg,
            "Show the song title as \"Listening to …\"",
            "",
            &mut cfg.discord.song_as_activity_name,
        );
    });
}

fn lyrics(ui: &mut Ui, pg: &mut Page, cfg: &mut Config) {
    group(ui, pg, Head::None, |ui, pg| {
        toggle(ui, pg, "Show lyrics", "", &mut cfg.lyrics.enabled);
        toggle(
            ui,
            pg,
            "Fetch lyrics online",
            "When there are no local lyrics: LRCLIB, then Genius.",
            &mut cfg.lyrics.online,
        );
    });
}

// ---------------------------------------------------------------------------------- playback

fn playback(ui: &mut Ui, pg: &mut Page, cfg: &mut Config) {
    group(ui, pg, Head::None, |ui, pg| {
        toggle(ui, pg, "Gapless playback", "", &mut cfg.playback.gapless);
        let help = if cfg.playback.crossfade > 0.0 {
            "Each song fades into the next. Albums stay gapless. Off in bit-perfect mode."
        } else {
            ""
        };
        row(ui, pg, "Crossfade", help, "fade", |ui| {
            ui.add(
                egui::Slider::new(&mut cfg.playback.crossfade, 0.0..=12.0)
                    .step_by(0.5)
                    .custom_formatter(|v, _| if v <= 0.0 { "Off".into() } else { format!("{v:.1} s") }),
            );
        });
        if cfg.playback.crossfade > 0.0 {
            toggle(
                ui,
                pg,
                "Also crossfade between songs of the same album",
                "",
                &mut cfg.playback.crossfade_albums,
            );
        }
        toggle(
            ui,
            pg,
            "Even out loudness",
            "Local files and SoundCloud play about as loud as Spotify. Off in bit-perfect mode.",
            &mut cfg.playback.replaygain,
        );
        if !cfg!(target_os = "ios") {
            toggle(
                ui,
                pg,
                "Open music videos in the player when they start",
                "Off: music videos start like songs, and the screen button in the player shows \
                 them.",
                &mut cfg.playback.youtube_video,
            );
        }
    });
}

fn audio(ui: &mut Ui, pg: &mut Page, cx: &mut Cx, cfg: &mut Config, st: &mut SettingsState) {
    group(ui, pg, Head::None, |ui, pg| {
        if !cfg!(target_os = "ios") {
            field(
                ui,
                pg,
                "mpv program",
                "Plays local files and SoundCloud.",
                &mut cfg.playback.mpv_path,
                "mpv",
                false,
            );
            if !st.devices_requested {
                st.devices_requested = true;
                cx.actions.push(Action::Cmd(Command::ListAudioDevices));
            }
            row(
                ui,
                pg,
                "Output device",
                "For local files and SoundCloud.",
                "sound card speakers",
                |ui| {
                    let current = cfg.playback.audio_device.clone();
                    let shown = if current.is_empty() {
                        "Automatic".to_string()
                    } else {
                        cx.feed
                            .audio_devices
                            .iter()
                            .find(|(id, _)| *id == current)
                            .map(|(_, d)| d.clone())
                            .unwrap_or(current)
                    };
                    egui::ComboBox::from_id_salt("audio-device")
                        .width(FIELD_W - 70.0)
                        .selected_text(shown)
                        .truncate()
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut cfg.playback.audio_device, String::new(), "Automatic");
                            for (id, desc) in cx.feed.audio_devices.iter().filter(|(id, _)| id != "auto") {
                                ui.selectable_value(
                                    &mut cfg.playback.audio_device,
                                    id.clone(),
                                    format!("{desc}  —  {id}"),
                                );
                            }
                        });
                    if ui.button("Refresh").clicked() {
                        cx.actions.push(Action::Cmd(Command::ListAudioDevices));
                    }
                },
            );
            toggle(
                ui,
                pg,
                "Bit-perfect output for lossless files",
                if cfg!(windows) {
                    "Lossless files reach your DAC untouched (WASAPI exclusive, no ReplayGain). \
                     Keep the volume at 100%."
                } else if cfg!(target_os = "macos") {
                    "Lossless files reach your DAC untouched at their own sample rate (hog mode, \
                     no ReplayGain). Keep the volume at 100%."
                } else {
                    "Lossless files reach your DAC untouched (no ReplayGain). Pick an \
                     \"alsa/hw:…\" device above and keep the volume at 100%. PipeWire resamples \
                     unless you allow more rates (see the README)."
                },
                &mut cfg.playback.bit_perfect,
            );
        }
        note(
            ui,
            pg,
            if cfg!(target_os = "ios") {
                "Spotify plays through librespot (320 kbps at most). iOS chooses the output."
            } else {
                "Spotify plays through librespot (320 kbps at most); everything else through mpv."
            },
            "",
        );
    });
}

fn downloads(ui: &mut Ui, pg: &mut Page, cx: &mut Cx, cfg: &mut Config) {
    group(ui, pg, Head::None, |ui, pg| {
        let root = cfg.library_root().to_string_lossy().into_owned();
        stack(
            ui,
            pg,
            "Download folder",
            "Empty: a folder for each service in your first music folder.",
            "browse",
            |ui| {
                ui.horizontal(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut cfg.downloads.folder)
                            .hint_text(format!("{root}/<service>"))
                            .desired_width(
                                ui.available_width() - buttons_width(ui, [theme::ic(icon::FOLDER_OPEN, "Browse…")]),
                            ),
                    );
                    if ui.button(theme::ic(icon::FOLDER_OPEN, "Browse…")).clicked() {
                        picker::open(ui.ctx(), "download-folder", "Download folder", Pick::Folder);
                    }
                });
            },
        );
        toggle(
            ui,
            pg,
            "Embed lyrics",
            "Time-synced when available.",
            &mut cfg.downloads.lyrics,
        );
        if cfg.server.enabled {
            let server = &cx.feed.server;
            let name = if server.name.is_empty() {
                "the media server"
            } else {
                server.name.as_str()
            };
            let now = match (server.allowed, server.uploads) {
                (_, true) => String::new(),
                (true, false) => format!(
                    " {name} doesn't take songs from this device yet: turn on “Let it upload here” for it \
                     in its dashboard (Music › Devices)."
                ),
                (false, false) => " It isn't connected now (see Media server).".to_string(),
            };
            toggle(
                ui,
                pg,
                "Send downloads to the media server",
                &format!(
                    "Downloads go into {name}'s music instead of this device. Saved here while it \
                     isn't connected.{now}"
                ),
                &mut cfg.downloads.to_server,
            );
        }
    });
    spotify_downloads(ui, pg, cfg);
    if !cfg!(target_os = "ios") {
        group(ui, pg, Head::Title("Music videos", ""), |ui, pg| {
            note(
                ui,
                pg,
                "Saved as video files, up to 1080p (MP4 when YouTube has it), in a Music videos \
                 folder.",
                "music videos youtube download video mp4",
            );
        });
    }
}

/// Spotify songs, saved straight from Spotify.
fn spotify_downloads(ui: &mut Ui, pg: &mut Page, cfg: &mut Config) {
    group(
        ui,
        pg,
        Head::Title(
            "Spotify songs",
            "Spotify's own Ogg Vorbis (320 kbps with Premium), tagged with everything Spotify \
             knows. Without a Spotify login, the same recording from SoundCloud.",
        ),
        |ui, pg| {
            if cfg!(target_os = "ios") {
                return;
            }
            toggle(
                ui,
                pg,
                "Save Spotify downloads as MP3 instead of Ogg Vorbis",
                "Converting can't add quality, but MP3 plays everywhere.",
                &mut cfg.downloads.spotify_mp3,
            );
        },
    );
}

// ---------------------------------------------------------------------------------- look

fn appearance(ui: &mut Ui, pg: &mut Page, cx: &mut Cx, cfg: &mut Config) {
    group(ui, pg, Head::Title("Theme", ""), |ui, pg| {
        stack(ui, pg, "", "", "theme colors colours holiday animation dark", |ui| {
            super::panels::theme_choices(ui, cx);
        });
        stack(ui, pg, "Accent colour", "", "color", |ui| {
            ui.horizontal_wrapped(|ui| {
                for (name, hex) in super::themes::ACCENTS {
                    let c = [(hex >> 16) as u8, (hex >> 8) as u8, hex as u8];
                    let (rect, resp) = ui.allocate_exact_size(egui::vec2(24.0, 24.0), egui::Sense::click());
                    let on = cfg.ui.accent == c;
                    ui.painter()
                        .circle_filled(rect.center(), 10.0, Color32::from_rgb(c[0], c[1], c[2]));
                    if on || resp.hovered() {
                        ui.painter()
                            .circle_stroke(rect.center(), 12.0, egui::Stroke::new(1.5, TEXT));
                    }
                    if resp.on_hover_text(name).clicked() {
                        cfg.ui.accent = c;
                    }
                }
                ui.color_edit_button_srgb(&mut cfg.ui.accent);
            });
        });
        let mut glass = !cfg.ui.classic_look;
        toggle(
            ui,
            pg,
            "Glass look",
            "Off: the classic look, flat and solid.",
            &mut glass,
        );
        cfg.ui.classic_look = !glass;
        toggle(
            ui,
            pg,
            "Colour backgrounds with the current cover",
            "",
            &mut cfg.ui.dynamic_accent,
        );
    });
    group(ui, pg, Head::Title("Window", ""), |ui, pg| {
        if cfg!(not(any(windows, target_os = "macos"))) {
            toggle(
                ui,
                pg,
                "Show a notification when a song starts",
                "",
                &mut cfg.ui.notifications,
            );
        }
        row(
            ui,
            pg,
            "Interface scale",
            "Ctrl + / Ctrl − / Ctrl 0",
            "zoom size",
            |ui| {
                let r = ui.add(egui::Slider::new(&mut cfg.ui.scale, super::SCALE.0..=super::SCALE.1).step_by(0.05));
                if r.drag_stopped() || r.lost_focus() {
                    ui.ctx().set_zoom_factor(cfg.ui.scale);
                }
            },
        );
        row(
            ui,
            pg,
            "Covers kept in memory",
            "Fewer cached covers = less RAM. Takes effect after a restart.",
            "cache",
            |ui| {
                ui.add(egui::DragValue::new(&mut cfg.ui.art_cache_size).range(32..=2000));
            },
        );
    });
}

// ---------------------------------------------------------------------------------- devices

fn media_server(ui: &mut Ui, pg: &mut Page, cx: &mut Cx, cfg: &mut Config) {
    let server = cx.feed.server.clone();
    group(ui, pg, Head::None, |ui, pg| {
        toggle(
            ui,
            pg,
            "Use a media server",
            "mediaserver on a Raspberry Pi or a computer that stays on: its music plays here, and \
             your settings and logins reach your other devices through it.",
            &mut cfg.server.enabled,
        );
        if !cfg.server.enabled {
            return;
        }
        let state = (!server.status.is_empty()).then(|| {
            (
                server.status.as_str(),
                if server.allowed { GREEN } else { pal().text_dim },
            )
        });
        row_with(
            ui,
            pg,
            "Address",
            state,
            "Empty: the one found on your network (or e.g. pi.local).",
            "",
            |ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut cfg.server.address)
                        .hint_text("Found on your network")
                        .desired_width(FIELD_W),
                );
            },
        );
        for (name, address) in &server.found {
            row(ui, pg, &format!("Found {name}"), address, "", |ui| {
                if cfg.server.address != *address && ui.small_button("Use this one").clicked() {
                    cfg.server.address = address.clone();
                }
            });
        }
        row(ui, pg, "This device", &server.device, "name", |_| {});
        toggle(
            ui,
            pg,
            "Play its music",
            "In a playlist named after it.",
            &mut cfg.server.library,
        );
    });
    if !cfg.server.enabled {
        return;
    }
    group(
        ui,
        pg,
        Head::Title(
            "Settings and logins",
            "Send from one device, Get on another. Only the server and the devices it lets in can \
             read them.",
        ),
        |ui, pg| {
            stack(ui, pg, "", "", "send get settings logins connect again", |ui| {
                ui.horizontal_wrapped(|ui| {
                    if ui
                        .add_enabled(server.allowed, egui::Button::new("Send my settings and logins"))
                        .clicked()
                    {
                        cx.actions.push(Action::Cmd(Command::ServerSendSettings));
                    }
                    if ui
                        .add_enabled(server.allowed, egui::Button::new("Get settings and logins from it"))
                        .clicked()
                    {
                        cx.actions.push(Action::Cmd(Command::ServerFetchSettings));
                    }
                    if ui.button("Connect again").clicked() {
                        cx.actions.push(Action::Cmd(Command::ServerConnect));
                    }
                });
            });
        },
    );
    #[cfg(not(target_os = "ios"))]
    if server.allowed {
        group(ui, pg, Head::Title("Upload", ""), |ui, pg| {
            server_upload(ui, pg, cx, &server)
        });
    }
}

/// Uploading songs to the media server: a folder or a song, with how far it's got. (What's
/// picked is taken in `take_picks`.)
#[cfg(not(target_os = "ios"))]
fn server_upload(ui: &mut Ui, pg: &mut Page, cx: &mut Cx, server: &crate::service::ServerFeed) {
    if let Some(up) = &server.upload {
        let what = if up.files == 0 {
            format!("Getting ready to upload {}…", up.file)
        } else {
            format!("Uploading {} ({} of {})", up.file, up.file_no, up.files)
        };
        stack(ui, pg, &what, "", "upload progress", |ui| {
            ui.horizontal(|ui| {
                let done = if up.total > 0 {
                    up.sent as f32 / up.total as f32
                } else {
                    0.0
                };
                widgets::progress(ui, done, (ui.available_width() - 80.0).max(120.0));
                if ui.button("Stop").clicked() {
                    cx.actions.push(Action::Cmd(Command::ServerUploadStop));
                }
            });
        });
    } else if server.uploads {
        stack(
            ui,
            pg,
            "Upload songs to it",
            "Folders keep their names. What's there already isn't sent again, and a stopped \
             upload carries on.",
            "upload folder song",
            |ui| {
                ui.horizontal_wrapped(|ui| {
                    if ui.button(theme::ic(icon::UPLOAD_SIMPLE, "Upload a folder…")).clicked() {
                        picker::open(
                            ui.ctx(),
                            "server-upload-folder",
                            "Upload a folder to the media server",
                            Pick::Folder,
                        );
                    }
                    if ui.button(theme::ic(icon::UPLOAD_SIMPLE, "Upload a song…")).clicked() {
                        picker::open(
                            ui.ctx(),
                            "server-upload-song",
                            "Upload a song to the media server",
                            Pick::File("Songs", crate::service::SERVER_SONG_EXTENSIONS),
                        );
                    }
                });
            },
        );
    } else {
        note(
            ui,
            pg,
            "To upload songs to it, turn on “Let it upload here” for this device in its dashboard \
             (Music › Devices).",
            "upload",
        );
    }
}

// ---------------------------------------------------------------------------------- app

fn updates(ui: &mut Ui, pg: &mut Page, cx: &mut Cx, cfg: &mut Config, paths: &Paths, rss_mb: f32) {
    use crate::service::UpdateStatus;
    let feed = cx.feed;
    let update = &feed.update;
    let busy = matches!(
        update.status,
        UpdateStatus::Checking
            | UpdateStatus::Downloading(_)
            | UpdateStatus::NeedsPassword { .. }
            | UpdateStatus::Installing
    );
    let state: Option<(String, Color32)> = match (&update.status, &update.available) {
        (UpdateStatus::NeedsPassword { .. }, _) => Some(("Waiting for your password…".into(), pal().text_dim)),
        (UpdateStatus::Installing, _) => Some(("Installing…".into(), pal().text_dim)),
        (UpdateStatus::Installed, _) => Some(("Installed. Start Utako again to use the new version.".into(), GREEN)),
        (UpdateStatus::Failed(why), _) => Some((why.clone(), DANGER)),
        (UpdateStatus::Checking | UpdateStatus::Downloading(_), _) => None,
        (_, Some(release)) => {
            let beta = if release.beta { " beta" } else { "" };
            Some((format!("Version {}{beta} is available", release.version), TEXT))
        }
        (UpdateStatus::UpToDate, None) => Some(("You have the newest version".into(), pal().text_dim)),
        _ => None,
    };
    let help = if !crate::updater::can_install() {
        "On Linux, update the way you installed: cd sumo-music && git pull && makepkg -sif"
    } else if crate::updater::NEEDS_PASSWORD {
        "Update now asks for your password and installs with pacman. Utako then restarts."
    } else {
        ""
    };
    group(ui, pg, Head::Title("Updates", ""), |ui, pg| {
        let version = format!("You have Utako {}", env!("CARGO_PKG_VERSION"));
        row_with(
            ui,
            pg,
            &version,
            state.as_ref().map(|(t, c)| (t.as_str(), *c)),
            help,
            "check now update install",
            |ui| {
                match &update.status {
                    UpdateStatus::Checking => {
                        ui.spinner();
                    }
                    UpdateStatus::Downloading(done) => {
                        widgets::progress(ui, *done, 160.0);
                    }
                    _ => {}
                }
                let settled = !busy && !matches!(update.status, UpdateStatus::Installed | UpdateStatus::Failed(_));
                if settled
                    && update.available.is_some()
                    && crate::updater::can_install()
                    && ui.button("Update now").clicked()
                {
                    cx.actions.push(Action::Cmd(Command::InstallUpdate));
                }
                if ui.add_enabled(!busy, egui::Button::new("Check now")).clicked() {
                    cx.actions.push(Action::Cmd(Command::CheckUpdates { manual: true }));
                }
            },
        );
        toggle(ui, pg, "Check for updates automatically", "", &mut cfg.updates.check);
        let beta_help = if cfg.updates.beta {
            "New versions first. They can have bugs."
        } else {
            "Turn on to get new versions first, as betas, before they go out to everyone."
        };
        toggle(ui, pg, "Beta updates", beta_help, &mut cfg.updates.beta);
    });
    group(ui, pg, Head::Title("About", ""), |ui, pg| {
        if pg.shows(&["Utako version about"]) {
            pg.begin(ui);
            egui::Frame::new().inner_margin(ROW_PAD).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    theme::logo(ui, cx.logo, 40.0);
                    ui.vertical(|ui| {
                        ui.label(RichText::new("Utako").font(theme::bold_font(18.0)));
                        ui.label(RichText::new(format!("Version {}", env!("CARGO_PKG_VERSION"))).color(pal().text_dim));
                    });
                });
            });
        }
        row(
            ui,
            pg,
            "Memory in use",
            "mpv runs as a separate process.",
            "ram",
            |ui| {
                ui.label(RichText::new(format!("{rss_mb:.0} MB")).color(pal().text_dim));
            },
        );
        stack(ui, pg, "Where Utako keeps its files", "", "config data folder", |ui| {
            ui.add(
                egui::Label::new(
                    RichText::new(format!("Config: {}", paths.config_file().display()))
                        .color(pal().text_faint)
                        .small(),
                )
                .wrap(),
            );
            ui.add(
                egui::Label::new(
                    RichText::new(format!("Data: {}", paths.data_dir.display()))
                        .color(pal().text_faint)
                        .small(),
                )
                .wrap(),
            );
        });
        stack(
            ui,
            pg,
            "Shortcuts",
            "Space play/pause · ←/→ seek · Ctrl+←/→ prev/next · ↑/↓ volume · Ctrl+K or / search · \
             Alt+←/→ back/forward · L lyrics view · Ctrl+F in Settings: search settings",
            "keyboard keys",
            |_| {},
        );
    });
}

fn backup(ui: &mut Ui, pg: &mut Page, cx: &mut Cx, st: &mut SettingsState) {
    group(
        ui,
        pg,
        Head::Title(
            "Export",
            "One file with everything in Settings. Import it on another computer to carry on \
             there.",
        ),
        |ui, pg| {
            let keys_help = if st.export_keys {
                "The file then holds your passwords and tokens: keep it private."
            } else {
                "Spotify, Last.fm, SoundCloud, Discord."
            };
            toggle(ui, pg, "Include keys and logins", keys_help, &mut st.export_keys);
            let mut playlists = !st.export_no_playlists;
            toggle(ui, pg, "Include your playlists and Liked Songs", "", &mut playlists);
            st.export_no_playlists = !playlists;
            if st.export_to.is_empty() {
                st.export_to = crate::backup::default_export_path().to_string_lossy().into_owned();
            }
            stack(ui, pg, "Save to", "", "export file backup", |ui| {
                ui.horizontal(|ui| {
                    let buttons = buttons_width(ui, [theme::ic(icon::UPLOAD_SIMPLE, "Export")]);
                    ui.add(egui::TextEdit::singleline(&mut st.export_to).desired_width(ui.available_width() - buttons));
                    if ui.button(theme::ic(icon::UPLOAD_SIMPLE, "Export")).clicked() && !st.export_to.trim().is_empty()
                    {
                        cx.actions.push(Action::Cmd(Command::ExportSettings {
                            path: expand_home(&st.export_to),
                            include: crate::backup::Include {
                                keys: st.export_keys,
                                playlists,
                            },
                        }));
                    }
                });
                if let Some(saved) = &cx.feed.exported_settings {
                    ui.horizontal(|ui| {
                        if ui.small_button("Show in folder").clicked() {
                            if let Some(dir) = saved.parent() {
                                cx.actions.push(Action::OpenUrl(dir.to_string_lossy().into_owned()));
                            }
                        }
                        let name = saved
                            .file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_default();
                        ui.add(
                            egui::Label::new(RichText::new(format!("Saved as {name}")).small().color(pal().text_dim))
                                .truncate(),
                        )
                        .on_hover_text(saved.display().to_string());
                    });
                }
            });
        },
    );
    if !st.settings_file_looked {
        st.settings_file_looked = true;
        if let Some(found) = crate::backup::find_settings_files().first() {
            st.settings_file = found.to_string_lossy().into_owned();
        }
    }
    if st.settings_file.is_empty() {
        if let Some(saved) = &cx.feed.exported_settings {
            st.settings_file = saved.to_string_lossy().into_owned();
        }
    }
    group(
        ui,
        pg,
        Head::Title(
            "Import",
            "Importing replaces your settings, adds the playlists and restarts Utako.",
        ),
        |ui, pg| {
            stack(
                ui,
                pg,
                "Settings file",
                "Or drop it onto the window.",
                "import restore",
                |ui| {
                    ui.horizontal(|ui| {
                        ui.add(
                            egui::TextEdit::singleline(&mut st.settings_file)
                                .hint_text("Settings file to import")
                                .desired_width(
                                    ui.available_width()
                                        - buttons_width(
                                            ui,
                                            [
                                                theme::ic(icon::FOLDER_OPEN, "Browse…"),
                                                theme::ic(icon::DOWNLOAD_SIMPLE, "Import…"),
                                            ],
                                        ),
                                ),
                        );
                        if ui.button(theme::ic(icon::FOLDER_OPEN, "Browse…")).clicked() {
                            picker::open(
                                ui.ctx(),
                                "settings-file",
                                "Settings file to import",
                                Pick::File("Utako settings", &["json"]),
                            );
                        }
                        let file = expand_home(&st.settings_file);
                        if ui
                            .add_enabled(
                                file.is_file(),
                                egui::Button::new(theme::ic(icon::DOWNLOAD_SIMPLE, "Import…")),
                            )
                            .clicked()
                        {
                            cx.actions.push(Action::ImportSettings(file));
                        }
                    });
                },
            );
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_matches_word_starts() {
        let q = words("Port");
        assert!(matches(&q, &["Login redirect port"]));
        assert!(!matches(&q, &["Import"]));
        assert!(matches(&words("m3u"), &["Import an .m3u playlist"]));
        assert!(!matches(&words("spotify gapless"), &["Spotify"]));
    }

    #[test]
    fn every_section_is_listed_once() {
        let listed: Vec<Sec> = GROUPS.iter().flat_map(|(_, s)| s.iter().copied()).collect();
        for (i, s) in listed.iter().enumerate() {
            assert!(!listed[i + 1..].contains(s), "{s:?} is listed twice");
            assert!(!s.info().label.is_empty() && !s.info().description.is_empty());
        }
        assert_eq!(listed.len(), 13);
    }
}
