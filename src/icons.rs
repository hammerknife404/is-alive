//! Line icons for the window's frame (the nav, the title bar), drawn with
//! the painter on a 24-point grid so they're crisp at any scale and take the
//! theme's colours. No icon font or crate: each is a handful of strokes.

use eframe::egui::{self, Color32, Pos2, Rect, Shape, Stroke, pos2};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Icon {
    Ping,
    Ports,
    Trace,
    Sweep,
    Lookup,
    Domains,
    Configs,
    Map,
    Notes,
    Speed,
    Calc,
    Results,
    Settings,
    Help,
    /// The nav's fold button: a panel with a chevron pointing the way it goes.
    Fold,
    Unfold,
}

/// One stroke on the 24-point grid.
enum Part {
    /// An open polyline.
    Line(&'static [(f32, f32)]),
    Circle(f32, f32, f32),
    /// Centre, radii.
    Ellipse(f32, f32, f32, f32),
    /// Centre, radius, from and to in degrees (0 is right, 90 is down).
    Arc(f32, f32, f32, f32, f32),
    /// A round dot, as a stroked point would be.
    Dot(f32, f32),
}

use Part::*;

fn parts(icon: Icon) -> &'static [Part] {
    match icon {
        Icon::Ping => &[Line(&[
            (3.0, 12.0),
            (7.0, 12.0),
            (10.0, 4.0),
            (14.0, 20.0),
            (17.0, 12.0),
            (21.0, 12.0),
        ])],
        Icon::Ports => &[
            Line(&[(9.0, 3.0), (9.0, 8.0)]),
            Line(&[(15.0, 3.0), (15.0, 8.0)]),
            Line(&[(6.0, 11.0), (6.0, 8.0), (18.0, 8.0), (18.0, 11.0)]),
            Arc(12.0, 11.0, 6.0, 0.0, 180.0),
            Line(&[(12.0, 17.0), (12.0, 21.0)]),
        ],
        Icon::Trace => &[
            Line(&[(4.0, 19.0), (14.0, 19.0)]),
            Arc(14.0, 15.5, 3.5, 90.0, -90.0),
            Line(&[(14.0, 12.0), (10.0, 12.0)]),
            Arc(10.0, 8.5, 3.5, 90.0, 270.0),
            Line(&[(10.0, 5.0), (20.0, 5.0)]),
        ],
        Icon::Sweep => &[
            Arc(12.0, 12.0, 9.0, 0.0, 270.0),
            Arc(12.0, 12.0, 4.5, 0.0, 270.0),
            Line(&[(12.0, 12.0), (18.5, 5.5)]),
        ],
        Icon::Lookup => &[
            Circle(12.0, 12.0, 9.0),
            Ellipse(12.0, 12.0, 3.8, 9.0),
            Line(&[(3.0, 12.0), (21.0, 12.0)]),
        ],
        Icon::Domains => &[
            Line(&[
                (3.0, 6.0),
                (21.0, 6.0),
                (21.0, 18.0),
                (3.0, 18.0),
                (3.0, 6.0),
            ]),
            Line(&[(3.0, 7.0), (12.0, 13.0), (21.0, 7.0)]),
        ],
        Icon::Configs => &[
            Line(&[
                (14.0, 3.0),
                (6.0, 3.0),
                (6.0, 21.0),
                (18.0, 21.0),
                (18.0, 7.0),
                (14.0, 3.0),
            ]),
            Line(&[(14.0, 3.0), (14.0, 7.0), (18.0, 7.0)]),
            Line(&[(10.0, 12.0), (8.0, 14.0), (10.0, 16.0)]),
            Line(&[(14.0, 12.0), (16.0, 14.0), (14.0, 16.0)]),
        ],
        Icon::Map => &[
            Line(&[
                (9.0, 4.0),
                (3.0, 6.0),
                (3.0, 20.0),
                (9.0, 18.0),
                (15.0, 20.0),
                (21.0, 18.0),
                (21.0, 4.0),
                (15.0, 6.0),
                (9.0, 4.0),
            ]),
            Line(&[(9.0, 4.0), (9.0, 18.0)]),
            Line(&[(15.0, 6.0), (15.0, 20.0)]),
        ],
        Icon::Notes => &[
            Line(&[
                (5.0, 4.0),
                (19.0, 4.0),
                (19.0, 20.0),
                (5.0, 20.0),
                (5.0, 4.0),
            ]),
            Line(&[(9.0, 9.0), (15.0, 9.0)]),
            Line(&[(9.0, 13.0), (15.0, 13.0)]),
            Line(&[(9.0, 17.0), (12.0, 17.0)]),
        ],
        Icon::Speed => &[
            Arc(12.0, 16.0, 8.0, 180.0, 360.0),
            Line(&[(12.0, 16.0), (16.0, 11.0)]),
            Dot(12.0, 16.0),
        ],
        Icon::Calc => &[
            Line(&[
                (6.0, 3.0),
                (18.0, 3.0),
                (18.0, 21.0),
                (6.0, 21.0),
                (6.0, 3.0),
            ]),
            Line(&[(9.0, 7.0), (15.0, 7.0)]),
            Dot(9.0, 12.0),
            Dot(12.0, 12.0),
            Dot(15.0, 12.0),
            Dot(9.0, 16.0),
            Dot(12.0, 16.0),
            Dot(15.0, 16.0),
        ],
        Icon::Results => &[
            Line(&[(9.0, 6.0), (20.0, 6.0)]),
            Line(&[(9.0, 12.0), (20.0, 12.0)]),
            Line(&[(9.0, 18.0), (20.0, 18.0)]),
            Dot(4.5, 6.0),
            Dot(4.5, 12.0),
            Dot(4.5, 18.0),
        ],
        Icon::Settings => &[
            Line(&[(4.0, 7.0), (13.0, 7.0)]),
            Line(&[(17.0, 7.0), (20.0, 7.0)]),
            Line(&[(4.0, 17.0), (7.0, 17.0)]),
            Line(&[(11.0, 17.0), (20.0, 17.0)]),
            Line(&[(15.0, 5.0), (15.0, 9.0)]),
            Line(&[(9.0, 15.0), (9.0, 19.0)]),
        ],
        Icon::Help => &[
            Circle(12.0, 12.0, 9.0),
            Arc(12.0, 9.5, 2.5, 180.0, 405.0),
            Line(&[(13.77, 11.27), (12.0, 12.8), (12.0, 14.0)]),
            Dot(12.0, 17.0),
        ],
        Icon::Fold => &[
            Line(&[
                (4.0, 4.0),
                (20.0, 4.0),
                (20.0, 20.0),
                (4.0, 20.0),
                (4.0, 4.0),
            ]),
            Line(&[(9.0, 4.0), (9.0, 20.0)]),
            Line(&[(15.5, 9.5), (13.0, 12.0), (15.5, 14.5)]),
        ],
        Icon::Unfold => &[
            Line(&[
                (4.0, 4.0),
                (20.0, 4.0),
                (20.0, 20.0),
                (4.0, 20.0),
                (4.0, 4.0),
            ]),
            Line(&[(9.0, 4.0), (9.0, 20.0)]),
            Line(&[(13.0, 9.5), (15.5, 12.0), (13.0, 14.5)]),
        ],
    }
}

/// Points around an ellipse arc, `from` to `to` degrees.
fn arc_points(
    at: impl Fn(f32, f32) -> Pos2,
    c: (f32, f32),
    r: (f32, f32),
    from: f32,
    to: f32,
) -> Vec<Pos2> {
    let steps = (((to - from).abs() / 360.0) * 40.0).ceil().max(6.0) as usize;
    (0..=steps)
        .map(|i| {
            let a = (from + (to - from) * i as f32 / steps as f32).to_radians();
            at(c.0 + r.0 * a.cos(), c.1 + r.1 * a.sin())
        })
        .collect()
}

/// Paints `icon` filling the square `rect`, in `colour`.
pub fn paint(painter: &egui::Painter, rect: Rect, icon: Icon, colour: Color32) {
    let s = rect.width().min(rect.height()) / 24.0;
    let o = rect.center() - egui::vec2(12.0 * s, 12.0 * s);
    let at = |x: f32, y: f32| pos2(o.x + x * s, o.y + y * s);
    let stroke = Stroke::new((1.75 * s).max(1.25), colour);
    for part in parts(icon) {
        let shape = match *part {
            Line(pts) => Shape::line(pts.iter().map(|&(x, y)| at(x, y)).collect(), stroke),
            Circle(x, y, r) => Shape::circle_stroke(at(x, y), r * s, stroke),
            Ellipse(x, y, rx, ry) => {
                let mut pts = arc_points(at, (x, y), (rx, ry), 0.0, 360.0);
                pts.pop();
                Shape::closed_line(pts, stroke)
            }
            Arc(x, y, r, from, to) => Shape::line(arc_points(at, (x, y), (r, r), from, to), stroke),
            Dot(x, y) => Shape::circle_filled(at(x, y), stroke.width * 0.65, colour),
        };
        painter.add(shape);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: [Icon; 16] = [
        Icon::Ping,
        Icon::Ports,
        Icon::Trace,
        Icon::Sweep,
        Icon::Lookup,
        Icon::Domains,
        Icon::Configs,
        Icon::Map,
        Icon::Notes,
        Icon::Speed,
        Icon::Calc,
        Icon::Results,
        Icon::Settings,
        Icon::Help,
        Icon::Fold,
        Icon::Unfold,
    ];

    /// Every stroke stays on the 24-point grid, so nothing spills out of
    /// the square an icon is given.
    #[test]
    fn icons_stay_inside_their_square() {
        let inside = |x: f32, y: f32| (0.0..=24.0).contains(&x) && (0.0..=24.0).contains(&y);
        for icon in ALL {
            assert!(!parts(icon).is_empty(), "{icon:?}");
            for part in parts(icon) {
                let ok = match *part {
                    Line(pts) => pts.len() >= 2 && pts.iter().all(|&(x, y)| inside(x, y)),
                    Circle(x, y, r) => inside(x - r, y - r) && inside(x + r, y + r),
                    Ellipse(x, y, rx, ry) => inside(x - rx, y - ry) && inside(x + rx, y + ry),
                    Arc(x, y, r, ..) => inside(x - r, y - r) && inside(x + r, y + r),
                    Dot(x, y) => inside(x, y),
                };
                assert!(ok, "{icon:?} leaves the grid");
            }
        }
    }
}
