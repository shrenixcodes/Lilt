//! The glass appearances' backdrop, drawn by Lilt itself.
//!
//! The desktop behind the island is read (see [`super::sample`]) at a resolution that
//! suits the chosen blur, from full resolution for clear glass to one eighth for a heavy
//! blur, and blurred further. After Slint renders a frame, that image is blended beneath
//! the translucent pixels of the island's surfaces, inside their rounded outlines and with
//! antialiased edges. Liquid Glass also bends it along the edges, the way a thick piece of
//! glass refracts what is behind it. Pixels outside the surfaces (the transparent
//! surroundings and soft shadows) are left alone, so they still show the desktop as it is.
//!
//! Everything happens in the island's own layered window: there is no second window to
//! keep aligned, and nothing depends on DWM's backdrop materials, which are not drawn
//! behind windows that never take the focus.

use super::surface::Bgra;

/// Premultiplied pixel target shared between the Win32 layered surface and test buffers.
pub trait PremulPixel: Copy {
    fn channels(&self) -> (u8, u8, u8, u8);
    fn set_channels(&mut self, r: u8, g: u8, b: u8, a: u8);
}

impl PremulPixel for Bgra {
    #[inline]
    fn channels(&self) -> (u8, u8, u8, u8) {
        (self.r, self.g, self.b, self.a)
    }

    #[inline]
    fn set_channels(&mut self, r: u8, g: u8, b: u8, a: u8) {
        self.r = r;
        self.g = g;
        self.b = b;
        self.a = a;
    }
}

/// The outline of one glass surface in physical pixels, relative to the island window.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shape {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub radius: f32,
    pub opacity: f32,
    pub border_width: f32,
    pub border_color: [u8; 4],
}

impl Shape {
    /// Scales a logical surface shape into physical pixels for display scale `scale`.
    pub fn scaled(self, scale: f32) -> Self {
        Self {
            x: self.x * scale,
            y: self.y * scale,
            width: self.width * scale,
            height: self.height * scale,
            radius: self.radius * scale,
            opacity: self.opacity,
            border_width: self.border_width * scale,
            border_color: self.border_color,
        }
    }
}

/// A copy of what lies behind the island window, possibly reduced and blurred.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Backdrop {
    pub width: usize,
    pub height: usize,
    /// Screen pixels per image pixel, in each direction.
    pub block: usize,
    /// Opaque sRGB colours, row by row.
    pub pixels: Vec<[u8; 3]>,
}

impl Backdrop {
    /// Blurs `pixels` with three passes of a box blur of `radius` (none for 0), which
    /// together approximate a Gaussian.
    pub fn new(width: usize, height: usize, block: usize, mut pixels: Vec<[u8; 3]>, radius: usize) -> Self {
        if width > 0 && height > 0 && pixels.len() == width * height && radius > 0 {
            for _ in 0..3 {
                box_blur(&mut pixels, width, height, radius);
            }
        }
        Self { width, height, block: block.max(1), pixels }
    }

    /// Bilinear sample at image coordinates (pixel centres at `i + 0.5`).
    fn sample(&self, u: f32, v: f32) -> [f32; 3] {
        let x = (u - 0.5).clamp(0.0, (self.width - 1) as f32);
        let y = (v - 0.5).clamp(0.0, (self.height - 1) as f32);
        let (x0, y0) = (x.floor() as usize, y.floor() as usize);
        let (x1, y1) = ((x0 + 1).min(self.width - 1), (y0 + 1).min(self.height - 1));
        let (fx, fy) = (x - x0 as f32, y - y0 as f32);
        let at = |x: usize, y: usize| self.pixels[y * self.width + x];
        let mut out = [0.0; 3];
        for (c, value) in out.iter_mut().enumerate() {
            let top = at(x0, y0)[c] as f32 * (1.0 - fx) + at(x1, y0)[c] as f32 * fx;
            let bottom = at(x0, y1)[c] as f32 * (1.0 - fx) + at(x1, y1)[c] as f32 * fx;
            *value = top * (1.0 - fy) + bottom * fy;
        }
        out
    }

    /// Averages over squares of about `size` screen pixels, the scale at which text must
    /// stay legible, and returns the brightest.
    pub fn brightest_area(&self, size: usize) -> Option<[u8; 3]> {
        let cells = (size / self.block).max(1);
        let mut best: Option<([u8; 3], u32)> = None;
        for by in (0..self.height).step_by(cells) {
            for bx in (0..self.width).step_by(cells) {
                let (mut sum, mut n) = ([0u32; 3], 0u32);
                for y in by..(by + cells).min(self.height) {
                    for x in bx..(bx + cells).min(self.width) {
                        let p = self.pixels[y * self.width + x];
                        for c in 0..3 {
                            sum[c] += u32::from(p[c]);
                        }
                        n += 1;
                    }
                }
                let mean = sum.map(|s| ((s + n / 2) / n.max(1)) as u8);
                // Rec. 709 weights on the encoded values: only used to rank areas.
                let light = 2126 * u32::from(mean[0]) + 7152 * u32::from(mean[1]) + 722 * u32::from(mean[2]);
                if best.is_none_or(|(_, l)| light > l) {
                    best = Some((mean, light));
                }
            }
        }
        best.map(|(colour, _)| colour)
    }
}

/// One box-blur pass along rows, then along columns, with a running sum, so its cost does
/// not grow with the radius. Edges repeat the outermost pixel.
fn box_blur(pixels: &mut [[u8; 3]], width: usize, height: usize, radius: usize) {
    let mut line: Vec<[u8; 3]> = Vec::new();
    let n = (2 * radius + 1) as u32;
    for (count, stride, lines, step) in [(width, 1, height, width), (height, width, width, 1)] {
        for l in 0..lines {
            let start = l * step;
            line.clear();
            line.extend((0..count).map(|i| pixels[start + i * stride]));
            let at = |i: isize| line[i.clamp(0, count as isize - 1) as usize];
            let mut sum = [0u32; 3];
            for k in -(radius as isize)..=radius as isize {
                let p = at(k);
                for c in 0..3 {
                    sum[c] += u32::from(p[c]);
                }
            }
            for i in 0..count {
                pixels[start + i * stride] = sum.map(|s| ((s + n / 2) / n) as u8);
                let (leaving, entering) = (at(i as isize - radius as isize), at(i as isize + radius as isize + 1));
                for c in 0..3 {
                    sum[c] = sum[c] + u32::from(entering[c]) - u32::from(leaving[c]);
                }
            }
        }
    }
}

/// The signed distance from (`x`, `y`) to the rounded rectangle's outline (negative
/// inside) and the outward direction of the nearest part of the outline.
fn outline(shape: &Shape, x: f32, y: f32) -> (f32, f32, f32) {
    let (hw, hh) = (shape.width / 2.0, shape.height / 2.0);
    let r = shape.radius.min(hw).min(hh).max(0.0);
    let (px, py) = (x - shape.x - hw, y - shape.y - hh);
    let (qx, qy) = (px.abs() - (hw - r), py.abs() - (hh - r));
    let (ox, oy) = (qx.max(0.0), qy.max(0.0));
    let outside = (ox * ox + oy * oy).sqrt();
    let distance = outside + qx.max(qy).min(0.0) - r;
    let (nx, ny) = if outside > 0.0 {
        (ox / outside, oy / outside)
    } else if qx > qy {
        (1.0, 0.0)
    } else {
        (0.0, 1.0)
    };
    (distance, nx * px.signum(), ny * py.signum())
}

/// How much of the pixel centred at (`x`, `y`) lies inside the rounded rectangle, 0..1.
#[cfg(test)]
fn coverage(shape: &Shape, x: f32, y: f32) -> f32 {
    (0.5 - outline(shape, x, y).0).clamp(0.0, 1.0)
}

/// Subpixel coverage `(outer, border)` in `0..1` for the pixel square centred at (`x`, `y`):
/// `outer` covers `distance <= 0`, and `border` covers `-border_width <= distance <= 0`.
fn edge_coverage(shape: &Shape, x: f32, y: f32, distance: f32) -> (f32, f32) {
    let bw = shape.border_width.max(0.0);
    let near_outer = distance.abs() <= 1.0;
    let near_inner = bw > 0.0 && (distance + bw).abs() <= 1.0;
    if !near_outer && !near_inner {
        let outer = if distance < 0.0 { 1.0 } else { 0.0 };
        let border = if bw > 0.0 && distance < 0.0 && distance >= -bw { 1.0 } else { 0.0 };
        return (outer, border);
    }
    let (hw, hh) = (shape.width / 2.0, shape.height / 2.0);
    let r = shape.radius.min(hw).min(hh).max(0.0);
    let on_corner = (x - shape.x - hw).abs() > hw - r - 1.0 && (y - shape.y - hh).abs() > hh - r - 1.0;
    if !on_corner {
        // Straight segments keep exact 1D analytic coverage so horizontal/vertical borders stay
        // sharp at integer scales and scale linearly at fractional DPI factors.
        let outer = (0.5 - distance).clamp(0.0, 1.0);
        let hole = if bw > 0.0 { (0.5 - (distance + bw)).clamp(0.0, 1.0) } else { 0.0 };
        return (outer, (outer - hole).max(0.0));
    }
    // 8x8 subpixel integration eliminates diagonal staircase steps and corner beads at 1x–2x DPI.
    const SAMPLES: usize = 8;
    const STEP: f32 = 1.0 / SAMPLES as f32;
    let mut outer_hits = 0u32;
    let mut border_hits = 0u32;
    for sy in 0..SAMPLES {
        let py = y - 0.5 + (sy as f32 + 0.5) * STEP;
        for sx in 0..SAMPLES {
            let px = x - 0.5 + (sx as f32 + 0.5) * STEP;
            let (d, _, _) = outline(shape, px, py);
            if d <= 0.0 {
                outer_hits += 1;
                if bw > 0.0 && d >= -bw {
                    border_hits += 1;
                }
            }
        }
    }
    let total = (SAMPLES * SAMPLES) as f32;
    (outer_hits as f32 / total, border_hits as f32 / total)
}

/// Blends `backdrop`, stretched over the whole window, beneath the pixels of `buffer`
/// (premultiplied BGRA, `width` pixels per row) inside `shapes`, within the given region.
///
/// With `refraction` above zero the backdrop bends along each outline: within a band as
/// wide as the corner radius, each pixel shows what lies further out, up to `refraction`
/// times the band's width at the edge, as a thick glass edge does.
pub fn compose(
    buffer: &mut [Bgra],
    width: usize,
    region: (usize, usize, usize, usize),
    shapes: &[Shape],
    backdrop: &Backdrop,
    refraction: f32,
) {
    let height = buffer.len() / width.max(1);
    if width == 0 || height == 0 || backdrop.width == 0 || backdrop.height == 0 {
        return;
    }
    let (sx, sy) = (backdrop.width as f32 / width as f32, backdrop.height as f32 / height as f32);
    let (left, top, right, bottom) = (region.0, region.1, region.2.min(width), region.3.min(height));
    for shape in shapes {
        let opacity = shape.opacity.clamp(0.0, 1.0);
        if opacity <= 0.0 {
            continue;
        }
        let target_alpha = opacity * 255.0;
        let band = (shape.radius.min(shape.width / 2.0).min(shape.height / 2.0) * 0.9).max(1.0);
        // Only rows and columns the shape (plus 1 px AA fringe) can touch.
        let x0 = left.max((shape.x - 1.0).floor().max(0.0) as usize);
        let x1 = right.min((shape.x + shape.width + 1.0).ceil().max(0.0) as usize);
        let y0 = top.max((shape.y - 1.0).floor().max(0.0) as usize);
        let y1 = bottom.min((shape.y + shape.height + 1.0).ceil().max(0.0) as usize);
        for y in y0..y1 {
            let row = &mut buffer[y * width..(y + 1) * width];
            let cy = y as f32 + 0.5;
            for (x, pixel) in row.iter_mut().enumerate().take(x1).skip(x0) {
                let cx = x as f32 + 0.5;
                let (distance, nx, ny) = outline(shape, cx, cy);
                let (cover, _) = edge_coverage(shape, cx, cy, distance);
                if cover <= 0.0 || pixel.a as f32 >= target_alpha {
                    continue;
                }
                let (mut u, mut v) = (cx, cy);
                let depth = -distance;
                if refraction > 0.0 && depth < band {
                    let k = 1.0 - depth.max(0.0) / band;
                    let shift = refraction * band * k * k;
                    u += nx * shift;
                    v += ny * shift;
                }
                // What the surface lets through, scaled by the surface's fade opacity so
                // fading glass never turns into a solid backdrop box mid-transition.
                let through = ((target_alpha - pixel.a as f32) / 255.0) * cover;
                let colour = backdrop.sample(u * sx, v * sy);
                let add = |channel: u8, value: f32| (channel as f32 + value * through).round().min(255.0) as u8;
                pixel.r = add(pixel.r, colour[0]);
                pixel.g = add(pixel.g, colour[1]);
                pixel.b = add(pixel.b, colour[2]);
                pixel.a = (pixel.a as f32 + 255.0 * through).round().min(255.0) as u8;
            }
        }
    }
}

/// Composites each shape's smooth SDF border over premultiplied `buffer` within `region`.
pub fn compose_borders<P: PremulPixel>(
    buffer: &mut [P],
    width: usize,
    region: (usize, usize, usize, usize),
    shapes: &[Shape],
) {
    let height = buffer.len() / width.max(1);
    if width == 0 || height == 0 {
        return;
    }
    let (left, top, right, bottom) = (region.0, region.1, region.2.min(width), region.3.min(height));
    for shape in shapes {
        let opacity = shape.opacity.clamp(0.0, 1.0);
        let bw = shape.border_width.max(0.0);
        let [br, bg, bb, ba] = shape.border_color;
        if opacity <= 0.0 || bw <= 0.0 || ba == 0 {
            continue;
        }
        let base_alpha = (ba as f32 / 255.0) * opacity;
        let x0 = left.max((shape.x - 1.0).floor().max(0.0) as usize);
        let x1 = right.min((shape.x + shape.width + 1.0).ceil().max(0.0) as usize);
        let y0 = top.max((shape.y - 1.0).floor().max(0.0) as usize);
        let y1 = bottom.min((shape.y + shape.height + 1.0).ceil().max(0.0) as usize);
        for y in y0..y1 {
            let row = &mut buffer[y * width..(y + 1) * width];
            let cy = y as f32 + 0.5;
            for (x, pixel) in row.iter_mut().enumerate().take(x1).skip(x0) {
                let cx = x as f32 + 0.5;
                let (distance, _, _) = outline(shape, cx, cy);
                if distance > 1.0 || distance < -(bw + 1.0) {
                    continue;
                }
                let (_, border_cov) = edge_coverage(shape, cx, cy, distance);
                if border_cov <= 0.0 {
                    continue;
                }
                let src_a = base_alpha * border_cov;
                let inv = 1.0 - src_a;
                let (dr, dg, db, da) = pixel.channels();
                let blend = |src: u8, dst: u8| (src as f32 * src_a + dst as f32 * inv).round().min(255.0) as u8;
                pixel.set_channels(
                    blend(br, dr),
                    blend(bg, dg),
                    blend(bb, db),
                    (255.0 * src_a + da as f32 * inv).round().min(255.0) as u8,
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shape(x: f32, y: f32, width: f32, height: f32, radius: f32) -> Shape {
        Shape { x, y, width, height, radius, opacity: 1.0, border_width: 0.0, border_color: [0; 4] }
    }

    #[test]
    fn coverage_follows_the_rounded_outline() {
        let s = shape(10.0, 10.0, 80.0, 40.0, 20.0);
        assert_eq!(coverage(&s, 50.0, 30.0), 1.0, "centre");
        assert_eq!(coverage(&s, 5.0, 30.0), 0.0, "left of it");
        assert_eq!(coverage(&s, 11.5, 11.5), 0.0, "the cut-away corner");
        assert_eq!(coverage(&s, 30.0, 10.5), 1.0, "the straight top edge, inside");
        let edge = coverage(&s, 30.0, 10.0);
        assert!(edge > 0.0 && edge < 1.0, "antialiased edge: {edge}");
    }

    #[test]
    fn the_outline_points_outwards() {
        let s = shape(0.0, 0.0, 100.0, 40.0, 10.0);
        let (_, nx, ny) = outline(&s, 50.0, 2.0);
        assert_eq!((nx, ny), (0.0, -1.0), "near the top edge");
        let (_, nx, ny) = outline(&s, 98.0, 20.0);
        assert_eq!((nx, ny), (1.0, 0.0), "near the right edge");
        let (_, nx, ny) = outline(&s, 2.0, 38.0);
        assert!(nx < 0.0 && ny > 0.0, "the lower left corner");
    }

    #[test]
    fn the_blur_spreads_and_keeps_flat_colour() {
        let flat = Backdrop::new(4, 3, 8, vec![[40, 80, 120]; 12], 2);
        assert!(flat.pixels.iter().all(|p| *p == [40, 80, 120]));
        let mut dot = vec![[0, 0, 0]; 9 * 9];
        dot[4 * 9 + 4] = [255, 255, 255];
        let blurred = Backdrop::new(9, 9, 8, dot.clone(), 1);
        let centre = blurred.pixels[4 * 9 + 4][0];
        assert!(centre > 0 && centre < 255);
        assert!(blurred.pixels[4 * 9 + 6][0] > 0, "spread two pixels away");
        assert!(blurred.pixels[4 * 9 + 6][0] < centre);
        // No blur: the image is kept as it is, for clear glass.
        assert_eq!(Backdrop::new(9, 9, 1, dot.clone(), 0).pixels, dot);
    }

    #[test]
    fn the_brightest_area_is_averaged_at_text_size() {
        // One bright pixel in a dark image: averaged over 16 pixels it is dim.
        let mut pixels = vec![[0, 0, 0]; 32 * 16];
        pixels[5 * 32 + 5] = [255, 255, 255];
        let sharp = Backdrop::new(32, 16, 1, pixels, 0);
        let area = sharp.brightest_area(16).unwrap();
        assert!(area[0] < 5, "{area:?}");
        // A bright half is found.
        let mut half = vec![[0, 0, 0]; 32 * 16];
        for y in 0..16 {
            for x in 16..32 {
                half[y * 32 + x] = [250, 250, 250];
            }
        }
        assert_eq!(Backdrop::new(32, 16, 1, half, 0).brightest_area(16), Some([250, 250, 250]));
    }

    #[test]
    fn the_desktop_shows_only_through_the_surfaces() {
        let (width, height) = (100, 60);
        // A translucent surface inside the shape, a faint shadow outside it.
        let mut buffer = vec![Bgra::default(); width * height];
        let glass = Bgra { b: 20, g: 20, r: 20, a: 128 };
        let shadow = Bgra { b: 0, g: 0, r: 0, a: 30 };
        for y in 0..height {
            for x in 0..width {
                let inside = coverage(&shape(10.0, 10.25, 80.0, 40.0, 20.0), x as f32 + 0.5, y as f32 + 0.5) > 0.0;
                buffer[y * width + x] = if inside { glass } else { shadow };
            }
        }
        let backdrop = Backdrop::new(2, 2, 50, vec![[200, 100, 0]; 4], 1);
        // Off the pixel grid, so the top edge crosses a row of pixels.
        let shapes = [shape(10.0, 10.25, 80.0, 40.0, 20.0)];
        compose(&mut buffer, width, (0, 0, width, height), &shapes, &backdrop, 0.0);
        let at = |x: usize, y: usize| buffer[y * width + x];
        // Inside: opaque, the surface over the desktop.
        assert_eq!(at(50, 30), Bgra { b: 20, g: 70, r: 120, a: 255 });
        // Outside and in the cut-away corner: untouched.
        assert_eq!(at(5, 5), shadow);
        assert_eq!(at(11, 11), shadow);
        // The antialiased edge is partly filled.
        let edge = at(50, 10);
        assert!(edge.a > 30 && edge.a < 255, "{edge:?}");
    }

    #[test]
    fn refraction_bends_the_backdrop_only_near_the_edges() {
        // The desktop: red on the left, blue on the right of x = 30.
        let (width, height) = (100, 40);
        let pixels: Vec<[u8; 3]> =
            (0..width * height).map(|i| if i % width < 30 { [255, 0, 0] } else { [0, 0, 255] }).collect();
        let backdrop = Backdrop::new(width, height, 1, pixels, 0);
        // A clear surface from x = 34, so its left edge sits over blue, just right of red.
        let surface = shape(34.0, 0.0, 60.0, 40.0, 16.0);
        let run = |refraction: f32| {
            let mut buffer = vec![Bgra::default(); width * height];
            compose(&mut buffer, width, (0, 0, width, height), &[surface], &backdrop, refraction);
            buffer
        };
        let flat = run(0.0);
        let bent = run(0.6);
        // Without refraction, the edge shows the blue directly behind it.
        assert_eq!(flat[20 * width + 35].r, 0);
        // With it, the edge shows the red further out; the middle is unchanged.
        assert!(bent[20 * width + 35].r > 200, "{:?}", bent[20 * width + 35]);
        assert_eq!(bent[20 * width + 64], flat[20 * width + 64]);
    }

    #[test]
    fn only_the_given_region_changes() {
        let (width, height) = (40, 20);
        let mut buffer = vec![Bgra { b: 0, g: 0, r: 0, a: 100 }; width * height];
        let backdrop = Backdrop::new(1, 1, 40, vec![[255, 255, 255]], 0);
        compose(&mut buffer, width, (0, 0, 20, 20), &[shape(0.0, 0.0, 40.0, 20.0, 0.0)], &backdrop, 0.0);
        assert_eq!(buffer[5 * width + 10].a, 255);
        assert_eq!(buffer[5 * width + 30].a, 100, "outside the region");
    }

    #[test]
    fn faded_glass_scales_backdrop_without_turning_opaque() {
        let (width, height) = (40, 20);
        let backdrop = Backdrop::new(1, 1, 40, vec![[200, 100, 0]], 0);

        let mut hidden = vec![Bgra::default(); width * height];
        let zero = Shape { opacity: 0.0, ..shape(0.0, 0.0, 40.0, 20.0, 0.0) };
        compose(&mut hidden, width, (0, 0, width, height), &[zero], &backdrop, 0.0);
        assert_eq!(hidden[5 * width + 10], Bgra::default());

        let mut half = vec![Bgra { b: 10, g: 10, r: 10, a: 64 }; width * height];
        let partial = Shape { opacity: 0.5, ..shape(0.0, 0.0, 40.0, 20.0, 0.0) };
        compose(&mut half, width, (0, 0, width, height), &[partial], &backdrop, 0.0);
        let pixel = half[5 * width + 10];
        assert!(pixel.a >= 127 && pixel.a <= 129, "half opacity target alpha: {pixel:?}");
        assert!(pixel.r > 10 && pixel.r < 120, "half backdrop contribution: {pixel:?}");
    }

    #[test]
    fn corner_borders_stay_uniform_along_diagonal_arcs() {
        let (width, height) = (96, 96);
        let mut buffer = vec![Bgra { b: 15, g: 15, r: 15, a: 255 }; width * height];
        let bordered = Shape {
            x: 8.0,
            y: 8.0,
            width: 80.0,
            height: 80.0,
            radius: 32.0,
            opacity: 1.0,
            border_width: 1.0,
            border_color: [255, 255, 255, 51],
        };
        compose_borders(&mut buffer, width, (0, 0, width, height), &[bordered]);
        // Every pixel must remain within single-layer border coverage (no 2x overlapping beads),
        // and integrated row energy along the 45-degree corner arc must vary monotonically without gaps.
        let max_corner = (10..28).map(|y| (8..32).map(|x| buffer[y * width + x].r).max().unwrap()).max().unwrap();
        assert!(max_corner <= 63, "border pixel exceeded single-layer blend ceiling: {max_corner}");
        let row_sums: Vec<u32> =
            (10..26).map(|y| (8..32).map(|x| u32::from(buffer[y * width + x].r - 15)).sum()).collect();
        for w in row_sums.windows(2) {
            assert!(w[0] + 6 >= w[1], "integrated corner stroke rose unexpectedly instead of tapering: {row_sums:?}");
        }
        assert_eq!(buffer[8 * width + 48].r, 63, "top straight edge keeps exact 1 px border");
        assert_eq!(buffer[9 * width + 48].r, 15, "row below top straight edge stays clean");
    }
}
