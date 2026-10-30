//! What a holiday theme draws: a soft light at the top of the page and art behind it (a moon,
//! stars, a blossom branch...), decorations over the page's top edge (a cobweb, Christmas
//! lights, paper lanterns) and an animation over everything (bats, snow, petals...).
//!
//! The animation runs at 20 frames a second, only while the window has focus, and never
//! during anything else: the window stays still (and costs nothing) when it's in the
//! background.

use std::f32::consts::{PI, TAU};

use egui::{pos2, vec2, Color32, Mesh, Painter, Pos2, Rect, Shape, Stroke};

use super::theme::with_alpha;
use super::themes::{Effect, Holiday};

const FRAME: f64 = 1.0 / 20.0;

/// One moving thing.
#[derive(Clone, Copy)]
struct Part {
    pos: Pos2,
    vel: egui::Vec2,
    size: f32,
    /// 0 far … 2 near.
    depth: f32,
    phase: f32,
    spin: f32,
    angle: f32,
    /// Seconds left (fireworks' sparks), or < 0 for "until it leaves".
    life: f32,
    color: u8,
}

/// The running animation.
pub struct Effects {
    effect: Option<Effect>,
    parts: Vec<Part>,
    /// Fireworks: rockets on their way up.
    rockets: Vec<Part>,
    last: Option<f64>,
    /// Seconds until the next bat or rocket.
    wait: f32,
    seed: u64,
    area: Rect,
}

impl Default for Effects {
    fn default() -> Self {
        Effects {
            effect: None,
            parts: Vec::new(),
            rockets: Vec::new(),
            last: None,
            wait: 1.0,
            seed: 0x9e37_79b9_7f4a_7c15,
            area: Rect::NOTHING,
        }
    }
}

impl Effects {
    fn rand(&mut self) -> f32 {
        // xorshift: plenty for snowflakes.
        self.seed ^= self.seed << 13;
        self.seed ^= self.seed >> 7;
        self.seed ^= self.seed << 17;
        (self.seed >> 40) as f32 / (1u64 << 24) as f32
    }

    fn range(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo) * self.rand()
    }

    /// Moves and draws `effect` over `area`. `running`: the window has focus, so it moves on.
    pub fn show(&mut self, ctx: &egui::Context, effect: Effect, area: Rect, running: bool) {
        if self.effect != Some(effect) || self.area.size() != area.size() {
            self.reset(effect, area);
        }
        self.area = area;
        let now = ctx.input(|i| i.time);
        let dt = match self.last {
            Some(t) if running => ((now - t) as f32).clamp(0.0, 0.1),
            _ => 0.0,
        };
        self.last = Some(now);
        if dt > 0.0 {
            self.step(dt, now as f32);
        }
        let painter = ctx
            .layer_painter(egui::LayerId::new(egui::Order::Middle, egui::Id::new("holiday-fx")))
            .with_clip_rect(area);
        self.draw(&painter, now as f32);
        if running {
            ctx.request_repaint_after(std::time::Duration::from_secs_f64(FRAME));
        }
    }

    fn reset(&mut self, effect: Effect, area: Rect) {
        self.effect = Some(effect);
        self.parts.clear();
        self.rockets.clear();
        self.wait = 1.0;
        self.seed = 0x9e37_79b9_7f4a_7c15;
        self.area = area;
        // Fewer on small windows: the counts are for a 1440×900 page.
        let scale = (area.width() * area.height() / (1200.0 * 760.0)).clamp(0.3, 1.6);
        let n = match effect {
            Effect::Snow => 70.0,
            Effect::Petals => 34.0,
            Effect::Hearts => 18.0,
            Effect::Fireflies => 28.0,
            Effect::Bats | Effect::Fireworks => 0.0,
        };
        for _ in 0..(n * scale) as usize {
            let p = self.spawn(effect, true);
            self.parts.push(p);
        }
    }

    /// A new particle: anywhere on the page at the start (`scatter`), else where it enters.
    fn spawn(&mut self, effect: Effect, scatter: bool) -> Part {
        let a = self.area;
        let depth = self.range(0.0, 2.0);
        let x = self.range(a.left(), a.right());
        let y = if scatter {
            self.range(a.top(), a.bottom())
        } else {
            a.top() - 10.0
        };
        let mut p = Part {
            pos: pos2(x, y),
            vel: vec2(0.0, 0.0),
            size: 1.0,
            depth,
            phase: self.range(0.0, TAU),
            spin: self.range(-1.5, 1.5),
            angle: self.range(0.0, TAU),
            life: -1.0,
            color: (self.rand() * 4.0) as u8,
        };
        match effect {
            Effect::Snow => {
                p.size = 0.9 + depth * 1.1 + self.range(0.0, 0.6);
                p.vel = vec2(0.0, 22.0 + depth * 24.0);
            }
            Effect::Petals => {
                p.size = 4.0 + depth * 2.2;
                p.vel = vec2(18.0, 26.0 + depth * 14.0);
            }
            Effect::Hearts => {
                p.size = 5.0 + depth * 4.0;
                p.vel = vec2(0.0, -(18.0 + depth * 14.0));
                if !scatter {
                    p.pos.y = a.bottom() + 12.0;
                }
            }
            Effect::Fireflies => {
                p.pos.y = self.range(a.top() + a.height() * 0.3, a.bottom());
                p.size = 1.6 + depth * 0.8;
                p.vel = vec2(self.range(-12.0, 12.0), self.range(-8.0, 8.0));
            }
            Effect::Bats => {
                let left = self.rand() < 0.5;
                p.size = 9.0 + depth * 6.5;
                p.pos = pos2(
                    if left { a.left() - 30.0 } else { a.right() + 30.0 },
                    self.range(a.top() + 30.0, a.top() + a.height() * 0.55),
                );
                let speed = 70.0 + depth * 40.0;
                p.vel = vec2(if left { speed } else { -speed }, self.range(-6.0, 6.0));
            }
            Effect::Fireworks => {}
        }
        p
    }

    fn step(&mut self, dt: f32, t: f32) {
        let Some(effect) = self.effect else { return };
        let a = self.area;
        // A slow, gusty wind for snow and petals.
        let wind = (t * 0.13).sin() * 14.0 + (t * 0.047).sin() * 9.0;
        let mut parts = std::mem::take(&mut self.parts);
        for p in parts.iter_mut() {
            match effect {
                Effect::Snow => {
                    p.pos.x += (wind * (0.4 + p.depth * 0.4) + (t * 0.8 + p.phase).sin() * 7.0) * dt;
                    p.pos.y += p.vel.y * dt;
                    p.angle += p.spin * dt * 0.4;
                }
                Effect::Petals => {
                    p.pos.x += (p.vel.x + wind * 1.4 + (t * 1.1 + p.phase).sin() * 12.0) * dt;
                    p.pos.y += p.vel.y * dt;
                    p.angle += p.spin * dt;
                }
                Effect::Hearts => {
                    p.pos.x += (t * 0.9 + p.phase).sin() * 14.0 * dt;
                    p.pos.y += p.vel.y * dt;
                }
                Effect::Fireflies => {
                    p.vel += vec2((t * 0.7 + p.phase).sin(), (t * 0.5 + p.phase * 1.7).cos()) * 6.0 * dt;
                    p.vel = p.vel.clamp(vec2(-16.0, -12.0), vec2(16.0, 12.0));
                    p.pos += p.vel * dt;
                    if p.pos.y < a.top() + a.height() * 0.25 || p.pos.y > a.bottom() {
                        p.vel.y = -p.vel.y;
                    }
                    if p.pos.x < a.left() || p.pos.x > a.right() {
                        p.vel.x = -p.vel.x;
                    }
                }
                Effect::Bats => {
                    p.pos.x += p.vel.x * dt;
                    p.pos.y += (p.vel.y + (t * 2.2 + p.phase).sin() * 18.0) * dt;
                }
                Effect::Fireworks => {
                    p.vel.y += 70.0 * dt;
                    p.vel *= 1.0 - 0.9 * dt;
                    p.pos += p.vel * dt;
                    p.life -= dt;
                }
            }
        }
        // Those that left come back where they enter; sparks and bats just go.
        let mut keep = Vec::with_capacity(parts.len());
        for p in parts {
            let gone = match effect {
                Effect::Snow | Effect::Petals => p.pos.y > a.bottom() + 12.0 || p.pos.x > a.right() + 40.0,
                Effect::Hearts => p.pos.y < a.top() - 20.0,
                Effect::Fireflies => false,
                Effect::Bats => p.pos.x < a.left() - 60.0 || p.pos.x > a.right() + 60.0,
                Effect::Fireworks => p.life <= 0.0,
            };
            if !gone {
                keep.push(p);
            } else if matches!(effect, Effect::Snow | Effect::Petals | Effect::Hearts) {
                let mut n = self.spawn(effect, false);
                if effect != Effect::Hearts && p.pos.x > a.right() {
                    n.pos = pos2(a.left() - 10.0, self.range(a.top(), a.bottom()));
                }
                keep.push(n);
            }
        }
        self.parts = keep;

        match effect {
            Effect::Bats => {
                self.wait -= dt;
                if self.wait <= 0.0 && self.parts.len() < 5 {
                    let b = self.spawn(Effect::Bats, false);
                    self.parts.push(b);
                    self.wait = self.range(1.5, 7.0);
                }
            }
            Effect::Fireworks => self.step_fireworks(dt),
            _ => {}
        }
    }

    fn step_fireworks(&mut self, dt: f32) {
        let a = self.area;
        self.wait -= dt;
        if self.wait <= 0.0 {
            self.wait = self.range(1.6, 3.6);
            let x = self.range(a.left() + a.width() * 0.15, a.right() - a.width() * 0.15);
            let peak = self.range(a.top() + a.height() * 0.12, a.top() + a.height() * 0.45);
            let color = (self.rand() * 4.0) as u8;
            let drift = self.range(-10.0, 10.0);
            self.rockets.push(Part {
                pos: pos2(x, a.bottom()),
                vel: vec2(drift, -((a.bottom() - peak) * 1.4).max(200.0)),
                size: peak,
                depth: 1.0,
                phase: 0.0,
                spin: 0.0,
                angle: 0.0,
                life: 3.0,
                color,
            });
        }
        let mut bursts = Vec::new();
        for r in self.rockets.iter_mut() {
            r.pos += r.vel * dt;
            r.vel.y += 120.0 * dt;
            if r.pos.y <= r.size || r.vel.y >= 0.0 {
                r.life = 0.0;
                bursts.push((r.pos, r.color));
            }
        }
        self.rockets.retain(|r| r.life > 0.0);
        for (at, color) in bursts {
            let n = self.range(36.0, 56.0) as usize;
            let ring = self.rand() < 0.3;
            for i in 0..n {
                let angle = i as f32 / n as f32 * TAU + self.range(-0.05, 0.05);
                let speed = if ring { 120.0 } else { self.range(40.0, 140.0) };
                let life = self.range(1.0, 1.8);
                let size = self.range(1.2, 2.2);
                self.parts.push(Part {
                    pos: at,
                    vel: vec2(angle.cos(), angle.sin()) * speed,
                    size,
                    depth: 1.0,
                    phase: 0.0,
                    spin: 0.0,
                    angle: 0.0,
                    life,
                    color,
                });
            }
        }
    }

    fn draw(&self, p: &Painter, t: f32) {
        let Some(effect) = self.effect else { return };
        for part in &self.parts {
            match effect {
                Effect::Snow => snowflake(p, part),
                Effect::Petals => petal(p, part),
                Effect::Hearts => {
                    // Fading out near the top.
                    let fade = ((part.pos.y - self.area.top()) / (self.area.height() * 0.4)).clamp(0.0, 1.0);
                    let colors = [0xf2557a, 0xfb7185, 0xf9a8d4, 0xe11d48];
                    let c = hex(colors[part.color as usize % 4]);
                    heart(
                        p,
                        part.pos,
                        part.size,
                        with_alpha(c, (fade * (110.0 + part.depth * 50.0)) as u8),
                    );
                }
                Effect::Fireflies => {
                    let blink = ((t * 0.9 + part.phase).sin() * 0.5 + 0.5).powf(2.0);
                    glow(p, part.pos, part.size * 5.0, hex(0xd9f99d), (blink * 60.0) as u8);
                    p.circle_filled(part.pos, part.size, with_alpha(hex(0xfef9c3), (blink * 230.0) as u8));
                }
                Effect::Bats => bat(p, part, t),
                Effect::Fireworks => {
                    let colors = [0xf5c542, 0xfb7185, 0x60a5fa, 0xfde68a];
                    let c = hex(colors[part.color as usize % 4]);
                    let a = (part.life / 1.4).clamp(0.0, 1.0);
                    p.line_segment(
                        [part.pos, part.pos - part.vel * 0.05],
                        Stroke::new(part.size, with_alpha(c, (a * 230.0) as u8)),
                    );
                }
            }
        }
        for r in &self.rockets {
            p.line_segment(
                [r.pos, r.pos - r.vel * 0.04],
                Stroke::new(1.6, with_alpha(hex(0xfde68a), 200)),
            );
        }
    }
}

fn hex(c: u32) -> Color32 {
    Color32::from_rgb((c >> 16) as u8, (c >> 8) as u8, c as u8)
}

/// A filled shape around `center` (which must see all of `outline`: hearts, petals, wings).
fn fan(p: &Painter, center: Pos2, outline: &[Pos2], color: Color32) {
    let mut m = Mesh::default();
    m.colored_vertex(center, color);
    for q in outline {
        m.colored_vertex(*q, color);
    }
    let n = outline.len() as u32;
    for i in 0..n {
        m.add_triangle(0, 1 + i, 1 + (i + 1) % n);
    }
    p.add(Shape::mesh(m));
}

/// A soft round light: strongest in the middle, nothing at `radius`.
pub fn glow(p: &Painter, center: Pos2, radius: f32, color: Color32, alpha: u8) {
    ellipse_glow(p, center, vec2(radius, radius), color, alpha);
}

fn ellipse_glow(p: &Painter, center: Pos2, radius: egui::Vec2, color: Color32, alpha: u8) {
    let mut m = Mesh::default();
    m.colored_vertex(center, with_alpha(color, alpha));
    let n = 32u32;
    for i in 0..n {
        let a = i as f32 / n as f32 * TAU;
        m.colored_vertex(
            center + vec2(a.cos() * radius.x, a.sin() * radius.y),
            Color32::TRANSPARENT,
        );
    }
    for i in 0..n {
        m.add_triangle(0, 1 + i, 1 + (i + 1) % n);
    }
    p.add(Shape::mesh(m));
}

fn snowflake(p: &Painter, s: &Part) {
    let alpha = (90.0 + s.depth * 70.0) as u8;
    let c = with_alpha(Color32::WHITE, alpha);
    // Some of the near ones are real six-armed crystals.
    if s.depth > 1.7 && s.color == 0 {
        let r = s.size * 2.2;
        for k in 0..3 {
            let a = s.angle + k as f32 * PI / 3.0;
            let d = vec2(a.cos(), a.sin()) * r;
            p.line_segment([s.pos - d, s.pos + d], Stroke::new(0.9, c));
        }
    } else {
        p.circle_filled(s.pos, s.size, c);
    }
}

fn petal(p: &Painter, s: &Part) {
    let colors = [0xfbcfe8, 0xf9a8d4, 0xfda4af, 0xffe4ec];
    let c = with_alpha(hex(colors[s.color as usize % 4]), (150.0 + s.depth * 45.0) as u8);
    // A teardrop with a notch at its wide end, turning as it falls.
    let (sin, cos) = (s.angle + s.phase).sin_cos();
    let flip = (s.angle * 1.7).cos().abs().max(0.25);
    let r = s.size;
    let local = [
        (0.0, -1.0),
        (0.55, -0.4),
        (0.6, 0.35),
        (0.25, 0.9),
        (0.0, 0.7),
        (-0.25, 0.9),
        (-0.6, 0.35),
        (-0.55, -0.4),
    ];
    let pts: Vec<Pos2> = local
        .iter()
        .map(|(x, y)| {
            let (x, y) = (x * r * flip, y * r);
            s.pos + vec2(x * cos - y * sin, x * sin + y * cos)
        })
        .collect();
    fan(p, s.pos, &pts, c);
}

fn heart(p: &Painter, at: Pos2, size: f32, color: Color32) {
    let pts: Vec<Pos2> = (0..28)
        .map(|i| {
            let t = i as f32 / 28.0 * TAU;
            let x = 16.0 * t.sin().powi(3);
            let y = 13.0 * t.cos() - 5.0 * (2.0 * t).cos() - 2.0 * (3.0 * t).cos() - (4.0 * t).cos();
            at + vec2(x, -y) * (size / 16.0)
        })
        .collect();
    fan(p, at, &pts, color);
}

fn bat(p: &Painter, b: &Part, t: f32) {
    // Dusty purple, so they show on the dark pages (Kumo's colour).
    let c = with_alpha(hex(0x8b7aa8), (130.0 + b.depth * 50.0) as u8);
    let s = b.size;
    let flap = (t * 11.0 + b.phase).sin();
    let dir = b.vel.x.signum();
    // Each wing: from the body out to a tip that rises and falls, with a scalloped edge.
    for side in [-1.0f32, 1.0] {
        let tip = b.pos + vec2(side * s * 1.25, -s * 0.55 * flap - s * 0.1);
        let elbow = b.pos + vec2(side * s * 0.6, -s * 0.35 * flap - s * 0.25);
        let edge1 = b.pos + vec2(side * s * 0.95, s * 0.15 - s * 0.2 * flap);
        let edge2 = b.pos + vec2(side * s * 0.45, s * 0.3);
        fan(
            p,
            b.pos + vec2(side * s * 0.3, 0.0),
            &[b.pos, elbow, tip, edge1, edge2],
            c,
        );
    }
    p.circle_filled(b.pos, s * 0.22, c);
    // Two ears, facing the way it flies.
    let head = b.pos + vec2(dir * s * 0.12, -s * 0.18);
    for e in [-1.0f32, 1.0] {
        p.add(Shape::convex_polygon(
            vec![
                head + vec2(e * s * 0.1, 0.0),
                head + vec2(e * s * 0.14, -s * 0.2),
                head + vec2(e * s * 0.02, 0.0),
            ],
            c,
            Stroke::NONE,
        ));
    }
}

/// The light from the top of the page and the art behind the pages (drawn before them).
pub fn backdrop(p: &Painter, page: Rect, h: &Holiday, t: f32, moving: bool) {
    // Light from the top, in the holiday's colour.
    let mut m = Mesh::default();
    let top = page.top();
    let bottom = top + 280.0f32.min(page.height());
    m.colored_vertex(pos2(page.left(), top), with_alpha(h.accent, 26));
    m.colored_vertex(pos2(page.right(), top), with_alpha(h.accent, 26));
    m.colored_vertex(pos2(page.left(), bottom), Color32::TRANSPARENT);
    m.colored_vertex(pos2(page.right(), bottom), Color32::TRANSPARENT);
    m.add_triangle(0, 1, 2);
    m.add_triangle(1, 2, 3);
    p.add(Shape::mesh(m));
    let twinkle = |phase: f32| {
        if moving {
            0.65 + 0.35 * (t * 1.7 + phase).sin()
        } else {
            1.0
        }
    };
    match h.id {
        "halloween" => {
            // A pale moon on the right, clear of the cobweb.
            let c = pos2(page.right() - 190.0, top + 130.0);
            glow(p, c, 120.0, hex(0xfde68a), 26);
            p.circle_filled(c, 44.0, with_alpha(hex(0xf5f0dc), 34));
            p.circle_filled(c + vec2(14.0, -8.0), 9.0, with_alpha(hex(0xd6cfb4), 22));
            p.circle_filled(c + vec2(-12.0, 12.0), 6.0, with_alpha(hex(0xd6cfb4), 20));
        }
        "newyear" | "summer" => {
            // A night sky: a few stars.
            let mut seed = 7u32;
            for i in 0..26 {
                seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12_345);
                let x = page.left() + (seed >> 8) as f32 % page.width().max(1.0);
                seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12_345);
                let y = top + 20.0 + (seed >> 8) as f32 % 220.0;
                let a = (40.0 * twinkle(i as f32)) as u8;
                let c = if h.id == "newyear" {
                    hex(0xf5c542)
                } else {
                    hex(0xe0f2fe)
                };
                p.circle_filled(pos2(x, y), 1.0 + (i % 3) as f32 * 0.5, with_alpha(c, a));
            }
        }
        "valentine" => {
            heart(
                p,
                pos2(page.left() + 70.0, page.bottom() - 90.0),
                60.0,
                with_alpha(h.accent, 14),
            );
            heart(
                p,
                pos2(page.right() - 90.0, top + 110.0),
                46.0,
                with_alpha(h.accent, 12),
            );
        }
        "blossom" => {
            // A cherry branch reaching in from the right.
            let brown = with_alpha(hex(0x6b4a3a), 120);
            let pts = [
                pos2(page.right(), top + 70.0),
                pos2(page.right() - 90.0, top + 92.0),
                pos2(page.right() - 170.0, top + 84.0),
                pos2(page.right() - 240.0, top + 104.0),
            ];
            p.add(Shape::line(pts.to_vec(), Stroke::new(3.0, brown)));
            p.line_segment([pts[1], pts[1] + vec2(-30.0, 30.0)], Stroke::new(2.0, brown));
            for (i, at) in [
                pts[1] + vec2(-30.0, 30.0),
                pts[2],
                pts[3],
                pts[1] + vec2(-8.0, -6.0),
                pts[2] + vec2(-34.0, 12.0),
            ]
            .into_iter()
            .enumerate()
            {
                blossom(p, at, 7.0 + (i % 2) as f32 * 2.0);
            }
        }
        _ => {}
    }
}

fn blossom(p: &Painter, at: Pos2, r: f32) {
    for k in 0..5 {
        let a = k as f32 / 5.0 * TAU - PI / 2.0;
        p.circle_filled(
            at + vec2(a.cos(), a.sin()) * r * 0.75,
            r * 0.55,
            with_alpha(hex(0xf9a8d4), 150),
        );
    }
    p.circle_filled(at, r * 0.28, with_alpha(hex(0xfde68a), 200));
}

/// What hangs over the page's top edge (drawn over it): a cobweb, Christmas lights or paper
/// lanterns.
pub fn garland(p: &Painter, page: Rect, h: &Holiday, t: f32, moving: bool) {
    let top = page.top();
    match h.id {
        "halloween" => {
            // A cobweb in the top right corner (the left one has the back and forward
            // buttons), its spider down a thread.
            let corner = page.right_top();
            let c = with_alpha(Color32::WHITE, 34);
            let len = 92.0;
            let spokes: Vec<egui::Vec2> = (0..6)
                .map(|i| {
                    let a = i as f32 / 5.0 * PI / 2.0;
                    vec2(-a.cos(), a.sin())
                })
                .collect();
            for d in &spokes {
                p.line_segment([corner, corner + *d * len], Stroke::new(1.0, c));
            }
            for ring in 1..5 {
                let r = len * ring as f32 / 4.6;
                let pts: Vec<Pos2> = spokes
                    .windows(2)
                    .flat_map(|w| {
                        // Each strand sags a little between two spokes.
                        let mid = (w[0] + w[1]).normalized() * r * 0.9;
                        [corner + w[0] * r, corner + mid]
                    })
                    .chain(std::iter::once(corner + spokes[5] * r))
                    .collect();
                p.add(Shape::line(pts, Stroke::new(1.0, c)));
            }
            let sway = if moving { (t * 0.8).sin() * 3.0 } else { 0.0 };
            let at = corner + vec2(-58.0 + sway, 118.0 + if moving { (t * 0.6).sin() * 6.0 } else { 0.0 });
            p.line_segment([corner + vec2(-58.0, 52.0), at], Stroke::new(1.0, c));
            let body = with_alpha(hex(0x120c16), 230);
            for leg in 0..4 {
                let y = -3.0 + leg as f32 * 2.2;
                for side in [-1.0f32, 1.0] {
                    p.line_segment(
                        [at + vec2(0.0, y), at + vec2(side * 8.0, y + 3.0 - leg as f32)],
                        Stroke::new(1.0, body),
                    );
                }
            }
            p.circle_filled(at, 4.5, body);
            p.circle_filled(at + vec2(0.0, -5.0), 3.0, body);
        }
        "christmas" => {
            // Lights on a wire that dips between hooks along the top.
            let hooks = ((page.width() / 220.0).round() as usize).max(2);
            let step = page.width() / hooks as f32;
            let colors = [0xef4444, 0x22c55e, 0xf5c542, 0x60a5fa];
            let wire = with_alpha(hex(0x2f3a2f), 220);
            let mut n = 0;
            for k in 0..hooks {
                let (x0, x1) = (page.left() + k as f32 * step, page.left() + (k + 1) as f32 * step);
                let sag = |f: f32| top + 1.0 + 9.0 * (PI * f).sin();
                let pts: Vec<Pos2> = (0..=16)
                    .map(|i| {
                        let f = i as f32 / 16.0;
                        pos2(x0 + (x1 - x0) * f, sag(f))
                    })
                    .collect();
                p.add(Shape::line(pts, Stroke::new(1.4, wire)));
                for b in 1..6 {
                    let f = b as f32 / 6.0;
                    let at = pos2(x0 + (x1 - x0) * f, sag(f) + 5.0);
                    let c = hex(colors[n % 4]);
                    let on = if moving {
                        0.55 + 0.45 * (t * 2.3 + n as f32 * 1.7).sin().abs()
                    } else {
                        1.0
                    };
                    glow(p, at, 9.0, c, (70.0 * on) as u8);
                    p.add(Shape::ellipse_filled(
                        at,
                        vec2(2.6, 3.6),
                        with_alpha(c, (255.0 * on) as u8),
                    ));
                    n += 1;
                }
            }
        }
        "summer" => {
            // Red paper lanterns, swaying a little.
            let n = ((page.width() / 260.0).round() as usize).max(2);
            for i in 0..n {
                let x = page.left() + (i as f32 + 0.5) * page.width() / n as f32;
                let sway = if moving { (t * 0.9 + i as f32).sin() * 3.0 } else { 0.0 };
                let at = pos2(x + sway, top + 30.0);
                p.line_segment(
                    [pos2(x, top), at - vec2(0.0, 11.0)],
                    Stroke::new(1.0, with_alpha(hex(0x3b2f2a), 200)),
                );
                glow(p, at, 34.0, hex(0xf97316), 40);
                p.add(Shape::ellipse_filled(at, vec2(10.0, 12.0), hex(0xc2410c)));
                p.rect_filled(
                    Rect::from_center_size(at - vec2(0.0, 12.0), vec2(9.0, 3.0)),
                    1.0,
                    hex(0x1f1a17),
                );
                p.rect_filled(
                    Rect::from_center_size(at + vec2(0.0, 12.0), vec2(9.0, 3.0)),
                    1.0,
                    hex(0x1f1a17),
                );
            }
        }
        _ => {}
    }
}
