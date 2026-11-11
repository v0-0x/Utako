//! A playlist's details: its name, description and picture (an image file, a link or an image
//! dropped on the window), for your own playlists.

use egui::{vec2, Color32, Key, RichText, Sense};

use super::art::ArtCache;
use super::picker::Cover;
use super::theme::{self, *};
use super::widgets;
use crate::model::Playlist;
use crate::service::Command;

pub struct PlaylistEditor {
    playlist: Playlist,
    name: String,
    description: String,
    cover: Cover,
    cover_text: String,
    note: Option<String>,
}

impl PlaylistEditor {
    pub fn open(playlist: Playlist) -> PlaylistEditor {
        PlaylistEditor {
            name: playlist.name.clone(),
            description: playlist.description.clone(),
            playlist,
            cover: Cover::Keep,
            cover_text: String::new(),
            note: None,
        }
    }

    /// What Save sends: `None` when nothing changed (or the name is empty).
    fn command(&self) -> Option<Command> {
        let name = self.name.trim();
        if name.is_empty() {
            return None;
        }
        let cover = self.cover.change();
        let description = self.description.trim();
        let changed = name != self.playlist.name || description != self.playlist.description.trim() || cover.is_some();
        changed.then(|| Command::EditPlaylist {
            playlist_id: self.playlist.id.clone(),
            name: name.to_string(),
            description: description.to_string(),
            cover,
        })
    }

    /// Draws the editor. Returns false once it is closed; what to save goes to `out`.
    pub fn show(&mut self, ctx: &egui::Context, art: &mut ArtCache, accent: Color32, out: &mut Vec<Command>) -> bool {
        // An image dropped on the window becomes the picture.
        if let Some(path) = super::picker::dropped_image(ctx) {
            self.cover = Cover::From(path);
        }
        let save_key = ctx.input(|i| i.modifiers.command && i.key_pressed(Key::Enter));
        let mut open = true;
        let modal = egui::Modal::new(egui::Id::new("playlist-editor")).show(ctx, |ui| {
            ui.set_width(520.0);
            ui.label(RichText::new("Edit playlist").font(theme::bold_font(18.0)));
            ui.add_space(10.0);
            ui.horizontal_top(|ui| {
                self.cover_box(ui, art, accent);
                ui.add_space(14.0);
                ui.vertical(|ui| {
                    ui.label(RichText::new("Name").small().color(pal().text_dim));
                    ui.add(egui::TextEdit::singleline(&mut self.name).desired_width(f32::INFINITY));
                    ui.add_space(8.0);
                    ui.label(RichText::new("Description").small().color(pal().text_dim));
                    ui.add(
                        egui::TextEdit::multiline(&mut self.description)
                            .hint_text("Add a description (optional)")
                            .desired_rows(5)
                            .desired_width(f32::INFINITY),
                    );
                });
            });
            if let Some(note) = &self.note {
                ui.add_space(4.0);
                ui.colored_label(theme::DANGER, note);
            }
            if self.name.trim().is_empty() {
                ui.add_space(4.0);
                ui.colored_label(theme::DANGER, "A playlist needs a name");
            }
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                let save = widgets::pill(ui, "Save", accent, theme::on_color(accent))
                    .on_hover_text("Ctrl+Enter")
                    .clicked();
                if (save || save_key) && !self.name.trim().is_empty() {
                    out.extend(self.command());
                    open = false;
                }
                if widgets::pill(ui, "Cancel", pal().card, TEXT).clicked() {
                    open = false;
                }
            });
        });
        if modal.should_close() {
            open = false;
        }
        open
    }

    fn cover_box(&mut self, ui: &mut egui::Ui, art: &mut ArtCache, accent: Color32) {
        ui.vertical(|ui| {
            ui.set_width(170.0);
            let (rect, resp) = ui.allocate_exact_size(vec2(170.0, 170.0), Sense::click());
            match &self.cover {
                Cover::Keep => widgets::playlist_cover(ui, art, &self.playlist, rect, 8),
                Cover::Remove => {
                    let shown = Playlist {
                        art: None,
                        ..self.playlist.clone()
                    };
                    widgets::playlist_cover(ui, art, &shown, rect, 8);
                }
                Cover::From(src) => {
                    widgets::cover(ui, art, Some(src), rect, 8, widgets::playlist_fallback(&self.playlist))
                }
            }
            if resp.on_hover_text("Choose a picture").clicked() {
                super::picker::open_image(ui.ctx(), "playlist-cover-file", "Playlist picture");
            }
            if self.cover != Cover::Keep {
                let text = if self.cover == Cover::Remove {
                    "No picture"
                } else {
                    "New picture"
                };
                let galley =
                    ui.painter()
                        .layout_no_wrap(text.to_string(), theme::bold_font(11.0), theme::on_color(accent));
                let pill = egui::Rect::from_min_size(rect.left_top() + vec2(8.0, 8.0), galley.size() + vec2(12.0, 6.0));
                ui.painter().rect_filled(pill, egui::CornerRadius::same(6), accent);
                ui.painter()
                    .galley(pill.min + vec2(6.0, 3.0), galley, theme::on_color(accent));
            }
            ui.add_space(6.0);
            ui.add(
                egui::TextEdit::singleline(&mut self.cover_text)
                    .hint_text("Image file or link")
                    .desired_width(170.0),
            );
            ui.horizontal(|ui| {
                let text = self.cover_text.trim().to_string();
                if ui.add_enabled(!text.is_empty(), egui::Button::new("Use")).clicked() {
                    self.cover = Cover::From(text);
                }
                if ui.button("Browse…").clicked() {
                    super::picker::open_image(ui.ctx(), "playlist-cover-file", "Playlist picture");
                }
                if (self.playlist.art.is_some() || matches!(self.cover, Cover::From(_)))
                    && ui.button("Remove").clicked()
                {
                    self.cover = Cover::Remove;
                }
            });
            match super::picker::take(ui.ctx(), "playlist-cover-file") {
                Some(Ok(p)) => {
                    self.cover_text = p.to_string_lossy().into_owned();
                    self.cover = Cover::From(self.cover_text.clone());
                    self.note = None;
                }
                Some(Err(e)) => self.note = Some(e),
                None => {}
            }
            ui.label(RichText::new("or drop an image here").small().color(pal().text_faint));
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::PlaylistKind;
    use crate::service::CoverChange;

    fn playlist() -> Playlist {
        Playlist {
            id: "custom:1".into(),
            name: "Mix".into(),
            kind: PlaylistKind::Custom,
            remote_id: None,
            description: String::new(),
            art: None,
            track_ids: Vec::new(),
        }
    }

    #[test]
    fn saves_only_what_changed() {
        let mut e = PlaylistEditor::open(playlist());
        assert!(e.command().is_none(), "nothing changed");
        e.name = "  Night drive ".into();
        e.description = "For the motorway\n".into();
        match e.command() {
            Some(Command::EditPlaylist {
                playlist_id,
                name,
                description,
                cover: None,
            }) => {
                assert_eq!((playlist_id.as_str(), name.as_str()), ("custom:1", "Night drive"));
                assert_eq!(description, "For the motorway");
            }
            _ => panic!("expected an edit"),
        }
        let mut e = PlaylistEditor::open(playlist());
        e.cover = Cover::From("/pics/a.png".into());
        assert!(matches!(
            e.command(),
            Some(Command::EditPlaylist {
                cover: Some(CoverChange::From(_)),
                ..
            })
        ));
        e.name = "   ".into();
        assert!(e.command().is_none(), "a playlist needs a name");
    }
}
