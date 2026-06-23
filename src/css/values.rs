//! Typed CSS value primitives — valid-by-construction.
//!
//! The cascade resolves CSS *declarations* (`property: value`) into typed
//! values here; layout + paint then consume those typed values directly,
//! never re-parsing a string. This is the core of the CSS typescape: a
//! [`Color`] is always in-range, a [`Length`] always carries its unit, a
//! [`Display`] is one of four known modes. There is no representable
//! "garbage color" or "length whose unit nobody knows" — every parser
//! returns [`None`] (or a typed default) on input it can't understand,
//! never a silently-wrong value.
//!
//! ## Unrepresentability tier (per the ★★ UNREPRESENTABILITY directive)
//!
//! - **[`Color`]** — *parse-time-rejected*. The struct fields are
//!   `pub` `f32` so an in-crate caller *could* build an out-of-range
//!   color, but the only public *ingress* from author input is
//!   [`Color::parse`], which returns `None` on anything it can't map to a
//!   valid sRGB-intent color. Every channel produced by `parse` /
//!   [`Color::TRANSPARENT`] is in `0.0..=1.0` by construction.
//! - **[`Length`]** — *parse-time-rejected*. [`Length::parse`] returns
//!   `None` for any token it can't classify into one of the known units
//!   (or `Auto`); a malformed length never reaches layout.
//! - **[`Display`]** — *truly-unrepresentable* off the parse boundary: the
//!   enum has exactly four variants, and [`Display::parse`] maps every
//!   unknown keyword to the CSS-default `Inline`. There is no "unknown
//!   display" value to represent.

/// An RGBA color, sRGB-intent, every channel in `0.0..=1.0`.
///
/// The consumer (a GPU renderer) is responsible for any sRGB→linear
/// conversion its target needs; these values are the parsed author intent.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Color {
    /// Red, `0.0..=1.0`.
    pub r: f32,
    /// Green, `0.0..=1.0`.
    pub g: f32,
    /// Blue, `0.0..=1.0`.
    pub b: f32,
    /// Alpha, `0.0..=1.0`.
    pub a: f32,
}

impl Color {
    /// Fully transparent — `(0, 0, 0, 0)`.
    pub const TRANSPARENT: Color = Color {
        r: 0.0,
        g: 0.0,
        b: 0.0,
        a: 0.0,
    };

    /// Construct an opaque color from `0..=255` integer channels (the
    /// natural form for hex / `rgb()` parsing). Private — the public
    /// ingress is [`Color::parse`].
    fn from_u8(r: u8, g: u8, b: u8) -> Color {
        Color {
            r: f32::from(r) / 255.0,
            g: f32::from(g) / 255.0,
            b: f32::from(b) / 255.0,
            a: 1.0,
        }
    }

    fn from_u8a(r: u8, g: u8, b: u8, a: f32) -> Color {
        Color {
            r: f32::from(r) / 255.0,
            g: f32::from(g) / 255.0,
            b: f32::from(b) / 255.0,
            a,
        }
    }

    /// Parse a CSS color string into a valid-by-construction [`Color`].
    ///
    /// Supported forms:
    /// - `#rgb` / `#rrggbb` / `#rrggbbaa` hex
    /// - `rgb(r, g, b)` / `rgba(r, g, b, a)` (`r`/`g`/`b` in `0..=255`,
    ///   `a` in `0.0..=1.0`)
    /// - a small named set (`black`, `white`, `red`, `green`, `blue`,
    ///   `yellow`, `cyan`, `magenta`, `gray`/`grey`, `transparent`, …)
    ///
    /// `currentColor` returns `None` (it resolves to the inherited `color`,
    /// which is not known here — the caller falls back). Any unrecognized
    /// or malformed input returns `None` — the **only** outcome on bad
    /// input is `None`, never a wrong color.
    #[must_use]
    pub fn parse(s: &str) -> Option<Color> {
        let s = s.trim();
        if s.is_empty() {
            return None;
        }
        if s.eq_ignore_ascii_case("currentColor") {
            return None;
        }
        if let Some(hex) = s.strip_prefix('#') {
            return parse_hex(hex);
        }
        // Functional forms: rgb(...) / rgba(...).
        if let Some(inner) = strip_func(s, "rgba").or_else(|| strip_func(s, "rgb")) {
            return parse_rgb_func(inner);
        }
        parse_named(s)
    }
}

/// Strip a `name(` … `)` wrapper case-insensitively, returning the inner
/// body. `None` if the string isn't that function form.
fn strip_func<'a>(s: &'a str, name: &str) -> Option<&'a str> {
    let lower = s.to_ascii_lowercase();
    let prefix = {
        let mut p = String::with_capacity(name.len() + 1);
        p.push_str(name);
        p.push('(');
        p
    };
    if lower.starts_with(&prefix) && lower.ends_with(')') {
        Some(&s[prefix.len()..s.len() - 1])
    } else {
        None
    }
}

/// Parse a hex body (no leading `#`): 3, 6, or 8 hex digits.
fn parse_hex(hex: &str) -> Option<Color> {
    if !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    match hex.len() {
        // #rgb → expand each nibble (f → ff).
        3 => {
            let r = dup_nibble(&hex[0..1])?;
            let g = dup_nibble(&hex[1..2])?;
            let b = dup_nibble(&hex[2..3])?;
            Some(Color::from_u8(r, g, b))
        }
        // #rrggbb.
        6 => {
            let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
            let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
            let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
            Some(Color::from_u8(r, g, b))
        }
        // #rrggbbaa.
        8 => {
            let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
            let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
            let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
            let a = u8::from_str_radix(&hex[6..8], 16).ok()?;
            Some(Color::from_u8a(r, g, b, f32::from(a) / 255.0))
        }
        _ => None,
    }
}

/// `"f"` → `0xff`. Input is one hex digit.
fn dup_nibble(one: &str) -> Option<u8> {
    let n = u8::from_str_radix(one, 16).ok()?;
    Some(n * 16 + n)
}

/// Parse the comma-separated body of `rgb(...)` / `rgba(...)`: 3 (opaque)
/// or 4 (with alpha) components. `r`/`g`/`b` are `0..=255` integers, `a`
/// is a `0.0..=1.0` float.
fn parse_rgb_func(inner: &str) -> Option<Color> {
    let parts: Vec<&str> = inner.split(',').map(str::trim).collect();
    match parts.len() {
        3 => {
            let r = parse_channel_u8(parts[0])?;
            let g = parse_channel_u8(parts[1])?;
            let b = parse_channel_u8(parts[2])?;
            Some(Color::from_u8(r, g, b))
        }
        4 => {
            let r = parse_channel_u8(parts[0])?;
            let g = parse_channel_u8(parts[1])?;
            let b = parse_channel_u8(parts[2])?;
            let a: f32 = parts[3].parse().ok()?;
            if !(0.0..=1.0).contains(&a) {
                return None;
            }
            Some(Color::from_u8a(r, g, b, a))
        }
        _ => None,
    }
}

/// Parse a `0..=255` channel value. Rejects out-of-range integers so a
/// `rgb(300, …)` is `None`, never a clamped wrong value.
fn parse_channel_u8(s: &str) -> Option<u8> {
    let v: u16 = s.parse().ok()?;
    if v > 255 {
        return None;
    }
    Some(v as u8)
}

/// A small set of CSS named colors — the common ones real pages use.
fn parse_named(s: &str) -> Option<Color> {
    let n = s.to_ascii_lowercase();
    let c = match n.as_str() {
        "transparent" => Color::TRANSPARENT,
        "black" => Color::from_u8(0, 0, 0),
        "white" => Color::from_u8(255, 255, 255),
        "red" => Color::from_u8(255, 0, 0),
        "green" => Color::from_u8(0, 128, 0),
        "lime" => Color::from_u8(0, 255, 0),
        "blue" => Color::from_u8(0, 0, 255),
        "yellow" => Color::from_u8(255, 255, 0),
        "cyan" | "aqua" => Color::from_u8(0, 255, 255),
        "magenta" | "fuchsia" => Color::from_u8(255, 0, 255),
        "gray" | "grey" => Color::from_u8(128, 128, 128),
        "silver" => Color::from_u8(192, 192, 192),
        "maroon" => Color::from_u8(128, 0, 0),
        "olive" => Color::from_u8(128, 128, 0),
        "navy" => Color::from_u8(0, 0, 128),
        "purple" => Color::from_u8(128, 0, 128),
        "teal" => Color::from_u8(0, 128, 128),
        "orange" => Color::from_u8(255, 165, 0),
        _ => return None,
    };
    Some(c)
}

/// Resolution context for a [`Length`]: the values a relative unit needs
/// to become a concrete pixel count.
#[derive(Debug, Clone, Copy)]
pub struct LengthContext {
    /// The element's own computed `font-size` (px) — basis for `em`.
    pub font_size: f32,
    /// The root element's `font-size` (px) — basis for `rem`.
    pub root_font_size: f32,
    /// Viewport width (px) — basis for `vw`.
    pub viewport_w: f32,
    /// Viewport height (px) — basis for `vh`.
    pub viewport_h: f32,
    /// The basis a `%` resolves against (px) — e.g. the containing
    /// block's width for `width: 50%`.
    pub percent_basis: f32,
}

impl LengthContext {
    /// A context with sensible defaults (16px font, the given viewport,
    /// percent basis = viewport width). The caller overrides per-axis.
    #[must_use]
    pub fn new(font_size: f32, root_font_size: f32, viewport_w: f32, viewport_h: f32) -> Self {
        Self {
            font_size,
            root_font_size,
            viewport_w,
            viewport_h,
            percent_basis: viewport_w,
        }
    }

    /// Return a copy with `percent_basis` set — chains per-axis at a call
    /// site (`ctx.with_percent_basis(parent_width)` for `width`).
    #[must_use]
    pub fn with_percent_basis(mut self, basis: f32) -> Self {
        self.percent_basis = basis;
        self
    }
}

/// A CSS length value — always carries its unit, valid-by-construction.
///
/// The [`Default`] is [`Length::Auto`] — the CSS initial value for the
/// box-sizing properties (`width`/`height`); per-property overrides
/// (margins/paddings start at `0px`) live in the consumer's default
/// construction, not here.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum Length {
    /// Absolute pixels.
    Px(f32),
    /// `em` — relative to the element's own font-size.
    Em(f32),
    /// `rem` — relative to the root font-size.
    Rem(f32),
    /// `%` — relative to a context basis.
    Percent(f32),
    /// `vw` — relative to viewport width.
    Vw(f32),
    /// `vh` — relative to viewport height.
    Vh(f32),
    /// `auto` — no fixed length; the layout engine decides. The
    /// [`Default`].
    #[default]
    Auto,
}

impl Length {
    /// Parse a CSS length token into a typed [`Length`].
    ///
    /// Accepts `auto`, a bare number (treated as `px` — common in author
    /// shorthand and what taffy-style layout expects), and `<n><unit>`
    /// for `px`/`em`/`rem`/`%`/`vw`/`vh`. Any token it can't classify
    /// returns `None` — never a wrong length.
    #[must_use]
    pub fn parse(s: &str) -> Option<Length> {
        let s = s.trim();
        if s.is_empty() {
            return None;
        }
        if s.eq_ignore_ascii_case("auto") {
            return Some(Length::Auto);
        }
        // Longest-suffix-first so "rem" wins over "em" / "m".
        for (suffix, ctor) in [
            ("rem", Length::Rem as fn(f32) -> Length),
            ("em", Length::Em as fn(f32) -> Length),
            ("px", Length::Px as fn(f32) -> Length),
            ("vw", Length::Vw as fn(f32) -> Length),
            ("vh", Length::Vh as fn(f32) -> Length),
            ("%", Length::Percent as fn(f32) -> Length),
        ] {
            if let Some(num) = s.strip_suffix(suffix) {
                // No inner trim: `s` is already trimmed at entry, so an
                // internal space (`"10 px"`) leaves a trailing space here
                // that `parse` rejects — malformed CSS stays `None`.
                let n: f32 = num.parse().ok()?;
                if !n.is_finite() {
                    return None;
                }
                return Some(ctor(n));
            }
        }
        // Bare number → px.
        let n: f32 = s.parse().ok()?;
        if !n.is_finite() {
            return None;
        }
        Some(Length::Px(n))
    }

    /// Resolve this length to a concrete pixel count against a context.
    ///
    /// Returns `None` for [`Length::Auto`] (the layout engine handles
    /// "auto" itself — there is no fixed pixel value). Every other variant
    /// resolves deterministically.
    #[must_use]
    pub fn resolve(self, ctx: &LengthContext) -> Option<f32> {
        match self {
            Length::Px(p) => Some(p),
            Length::Em(e) => Some(e * ctx.font_size),
            Length::Rem(r) => Some(r * ctx.root_font_size),
            Length::Percent(p) => Some(p / 100.0 * ctx.percent_basis),
            Length::Vw(v) => Some(v / 100.0 * ctx.viewport_w),
            Length::Vh(v) => Some(v / 100.0 * ctx.viewport_h),
            Length::Auto => None,
        }
    }
}

/// The CSS `display` mode — the four the layout engine distinguishes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Display {
    /// Inline flow (the CSS default for unknown elements).
    #[default]
    Inline,
    /// Block flow.
    Block,
    /// Flex container.
    Flex,
    /// Not rendered — no box generated.
    None,
}

impl Display {
    /// Parse a `display` keyword. Unknown / unsupported keywords map to
    /// the CSS default [`Display::Inline`] — there is no representable
    /// "unknown display".
    #[must_use]
    pub fn parse(s: &str) -> Display {
        match s.trim().to_ascii_lowercase().as_str() {
            "block" => Display::Block,
            "flex" => Display::Flex,
            "none" => Display::None,
            // "inline", "inline-block", anything unrecognized → inline.
            _ => Display::Inline,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-5
    }

    // ── Color::parse — hex ───────────────────────────────────────────

    #[test]
    fn color_hex_rrggbb() {
        let c = Color::parse("#3050ff").unwrap();
        assert!(approx(c.r, 0x30 as f32 / 255.0));
        assert!(approx(c.g, 0x50 as f32 / 255.0));
        assert!(approx(c.b, 1.0));
        assert!(approx(c.a, 1.0));
    }

    #[test]
    fn color_hex_short_rgb() {
        // #f00 → #ff0000 red.
        let c = Color::parse("#f00").unwrap();
        assert_eq!((c.r, c.g, c.b, c.a), (1.0, 0.0, 0.0, 1.0));
        // #fff → white.
        let w = Color::parse("#fff").unwrap();
        assert_eq!((w.r, w.g, w.b, w.a), (1.0, 1.0, 1.0, 1.0));
    }

    #[test]
    fn color_hex_rrggbbaa() {
        // 50% alpha (0x80 / 255 ≈ 0.502).
        let c = Color::parse("#ff000080").unwrap();
        assert!(approx(c.r, 1.0));
        assert!(approx(c.a, f32::from(0x80u8) / 255.0));
    }

    #[test]
    fn color_hex_white_and_black() {
        let w = Color::parse("#ffffff").unwrap();
        assert_eq!((w.r, w.g, w.b, w.a), (1.0, 1.0, 1.0, 1.0));
        let k = Color::parse("#000000").unwrap();
        assert_eq!((k.r, k.g, k.b, k.a), (0.0, 0.0, 0.0, 1.0));
    }

    // ── Color::parse — rgb()/rgba() ──────────────────────────────────

    #[test]
    fn color_rgb_func() {
        let c = Color::parse("rgb(255, 0, 0)").unwrap();
        assert_eq!((c.r, c.g, c.b, c.a), (1.0, 0.0, 0.0, 1.0));
    }

    #[test]
    fn color_rgba_func_translucent() {
        let c = Color::parse("rgba(255, 0, 0, 0.50)").unwrap();
        assert!(approx(c.r, 1.0));
        assert!(approx(c.a, 0.5));
    }

    #[test]
    fn color_rgba_no_spaces() {
        let c = Color::parse("rgba(10,20,30,1)").unwrap();
        assert!(approx(c.r, 10.0 / 255.0));
        assert!(approx(c.a, 1.0));
    }

    #[test]
    fn color_rgb_case_insensitive_func_name() {
        assert!(Color::parse("RGB(0,0,0)").is_some());
        assert!(Color::parse("RgBa(0,0,0,1)").is_some());
    }

    // ── Color::parse — named + currentColor + TRANSPARENT ────────────

    #[test]
    fn color_named_set() {
        assert_eq!(Color::parse("red"), Some(Color::from_u8(255, 0, 0)));
        assert_eq!(Color::parse("black"), Some(Color::from_u8(0, 0, 0)));
        assert_eq!(Color::parse("white"), Some(Color::from_u8(255, 255, 255)));
        assert_eq!(Color::parse("BLUE"), Some(Color::from_u8(0, 0, 255)));
        assert_eq!(Color::parse("transparent"), Some(Color::TRANSPARENT));
        assert_eq!(Color::parse("grey"), Color::parse("gray"));
    }

    #[test]
    fn color_current_color_is_none() {
        assert_eq!(Color::parse("currentColor"), None);
        assert_eq!(Color::parse("currentcolor"), None);
    }

    #[test]
    fn transparent_const_is_zero_alpha() {
        assert_eq!(Color::TRANSPARENT.a, 0.0);
    }

    // ── Color::parse — illegal → None, never wrong ───────────────────

    #[test]
    fn color_rejects_malformed() {
        assert_eq!(Color::parse(""), None);
        assert_eq!(Color::parse("   "), None);
        assert_eq!(Color::parse("#gg0000"), None); // non-hex digit
        assert_eq!(Color::parse("#12345"), None); // 5 digits (not 3/6/8)
        assert_eq!(Color::parse("#1234567"), None); // 7 digits
        assert_eq!(Color::parse("notacolor"), None);
        assert_eq!(Color::parse("rgb(300, 0, 0)"), None); // channel > 255
        assert_eq!(Color::parse("rgba(0, 0, 0, 2)"), None); // alpha > 1
        assert_eq!(Color::parse("rgb(0, 0)"), None); // wrong arity
        assert_eq!(Color::parse("rgb(0, 0, 0, 0, 0)"), None); // wrong arity
    }

    #[test]
    fn color_channels_always_in_range() {
        // Every successful parse yields in-range channels.
        for s in [
            "#abcdef",
            "rgb(1,2,3)",
            "rgba(254,1,9,0.3)",
            "orange",
            "#0a0b0c0d",
        ] {
            let c = Color::parse(s).unwrap();
            for ch in [c.r, c.g, c.b, c.a] {
                assert!((0.0..=1.0).contains(&ch), "{s} produced out-of-range {ch}");
            }
        }
    }

    // ── Length::parse ────────────────────────────────────────────────

    #[test]
    fn length_parse_px() {
        assert_eq!(Length::parse("100px"), Some(Length::Px(100.0)));
        assert_eq!(Length::parse(" 25px "), Some(Length::Px(25.0)));
    }

    #[test]
    fn length_parse_bare_number_is_px() {
        assert_eq!(Length::parse("50"), Some(Length::Px(50.0)));
        assert_eq!(Length::parse("0"), Some(Length::Px(0.0)));
    }

    #[test]
    fn length_parse_em_rem() {
        assert_eq!(Length::parse("2em"), Some(Length::Em(2.0)));
        // rem must win over em (longest-suffix-first).
        assert_eq!(Length::parse("1.5rem"), Some(Length::Rem(1.5)));
    }

    #[test]
    fn length_parse_percent() {
        assert_eq!(Length::parse("50%"), Some(Length::Percent(50.0)));
        assert_eq!(Length::parse("100%"), Some(Length::Percent(100.0)));
    }

    #[test]
    fn length_parse_viewport_units() {
        assert_eq!(Length::parse("50vw"), Some(Length::Vw(50.0)));
        assert_eq!(Length::parse("75vh"), Some(Length::Vh(75.0)));
    }

    #[test]
    fn length_parse_auto() {
        assert_eq!(Length::parse("auto"), Some(Length::Auto));
        assert_eq!(Length::parse("AUTO"), Some(Length::Auto));
    }

    #[test]
    fn length_parse_negative() {
        // Negative margins are legal CSS.
        assert_eq!(Length::parse("-10px"), Some(Length::Px(-10.0)));
        assert_eq!(Length::parse("-2em"), Some(Length::Em(-2.0)));
    }

    #[test]
    fn length_rejects_garbage() {
        assert_eq!(Length::parse(""), None);
        assert_eq!(Length::parse("   "), None);
        assert_eq!(Length::parse("abc"), None);
        assert_eq!(Length::parse("10pt"), None); // unsupported unit
        assert_eq!(Length::parse("px"), None); // no number
        assert_eq!(Length::parse("10 px"), None); // space inside
        assert_eq!(Length::parse("NaN"), None);
        assert_eq!(Length::parse("infpx"), None); // non-finite
    }

    // ── Length::resolve ──────────────────────────────────────────────

    fn ctx() -> LengthContext {
        // font 20, root 16, viewport 1000x500, percent basis 200.
        LengthContext {
            font_size: 20.0,
            root_font_size: 16.0,
            viewport_w: 1000.0,
            viewport_h: 500.0,
            percent_basis: 200.0,
        }
    }

    #[test]
    fn resolve_px_is_identity() {
        assert_eq!(Length::Px(42.0).resolve(&ctx()), Some(42.0));
    }

    #[test]
    fn resolve_em_uses_font_size() {
        assert_eq!(Length::Em(2.0).resolve(&ctx()), Some(40.0));
    }

    #[test]
    fn resolve_rem_uses_root_font_size() {
        assert_eq!(Length::Rem(2.0).resolve(&ctx()), Some(32.0));
    }

    #[test]
    fn resolve_percent_uses_basis() {
        assert_eq!(Length::Percent(50.0).resolve(&ctx()), Some(100.0));
    }

    #[test]
    fn resolve_vw_vh() {
        assert_eq!(Length::Vw(50.0).resolve(&ctx()), Some(500.0));
        assert_eq!(Length::Vh(50.0).resolve(&ctx()), Some(250.0));
    }

    #[test]
    fn resolve_auto_is_none() {
        assert_eq!(Length::Auto.resolve(&ctx()), None);
    }

    #[test]
    fn length_context_with_percent_basis_overrides() {
        let c = LengthContext::new(16.0, 16.0, 800.0, 600.0).with_percent_basis(400.0);
        assert_eq!(Length::Percent(25.0).resolve(&c), Some(100.0));
    }

    // ── Display::parse ───────────────────────────────────────────────

    #[test]
    fn display_parse_known() {
        assert_eq!(Display::parse("block"), Display::Block);
        assert_eq!(Display::parse("flex"), Display::Flex);
        assert_eq!(Display::parse("none"), Display::None);
        assert_eq!(Display::parse("inline"), Display::Inline);
    }

    #[test]
    fn display_parse_case_insensitive() {
        assert_eq!(Display::parse("BLOCK"), Display::Block);
        assert_eq!(Display::parse("  Flex  "), Display::Flex);
    }

    #[test]
    fn display_parse_unknown_is_inline() {
        assert_eq!(Display::parse("grid"), Display::Inline);
        assert_eq!(Display::parse("inline-block"), Display::Inline);
        assert_eq!(Display::parse("garbage"), Display::Inline);
        assert_eq!(Display::parse(""), Display::Inline);
    }

    #[test]
    fn display_default_is_inline() {
        assert_eq!(Display::default(), Display::Inline);
    }
}
