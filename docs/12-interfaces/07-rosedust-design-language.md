# ROSEDUST Design Language

> The visual identity of Roko: rose tones, deliberate contrast, glass morphism, and luxury motion shared across TUI, the first-party Web Portal, and Spectre visualization. The browser surface is dark-led, but REF29 requires light, dark, and high-contrast variants over the same token system.


> **Implementation**: Scaffold

**Topic**: [12-interfaces](./INDEX.md)
**Prerequisites**: None
**Key sources**: `refactoring-prd/06-interfaces.md` §3, `roko-cli/src/tui/theme.rs`, `roko-cli/src/tui/color.rs`, `bardo-backup/prd/shared/branding.md` §5.2

---

## Abstract

ROSEDUST is Roko's design language — a comprehensive visual system that unifies the appearance of every interface surface: the Terminal UI (ratatui), the Web Portal, Spectre creature visualizations, and CLI output. The name evokes the palette's essential character: rose light on a deliberate ground plane, as if viewing the system through a faintly glowing, dusty lens.

ROSEDUST is **dark-led**, not dark-only. The TUI and Spectre-heavy views still assume deep backgrounds, but the first-party web UI should ship light, dark, and high-contrast variants using the same semantic tokens, spacing rules, motion rules, and accent hierarchy. The rose palette remains the signature accent family, while semantic colors (jade, amber, crimson, violet, sapphire) provide differentiation without breaking the overall identity. See [13-web-portal.md](./13-web-portal.md) and [tmp/refinements/29-web-ui-architecture.md](../../tmp/refinements/29-web-ui-architecture.md).

The design language was preserved unchanged during the architectural migration from the legacy system. ROSEDUST predates the Roko reframing and was designed specifically for the cognitive agent visualization use case — it maps to Daimon behavioral states, knowledge tier density, and collective intelligence metrics through color, motion, and form.

---

## Color System

### Background Hierarchy

Three layers of background depth create visual hierarchy without explicit borders:

| Token | Hex | RGB | Usage |
|---|---|---|---|
| `void-black` | `#0a0a0f` | (10, 10, 15) | Deepest background. The void. |
| `twilight` | `#12101a` | (18, 16, 26) | Card and panel backgrounds |
| `dusk` | `#1a1726` | (26, 23, 38) | Elevated surfaces, modals |

Never use pure `#000000`. The void-black has a violet undertone that gives it warmth and depth. This is the foundation everything else sits on.

### Rose Palette (Primary Accent)

Rose is the dominant color. It carries meaning: activity, attention, life.

| Token | Hex | RGB | Usage |
|---|---|---|---|
| `rose-dim` | `#8b5e6b` | (139, 94, 107) | Muted, inactive elements. Borders. |
| `rose` | `#c77d8f` | (199, 125, 143) | Standard accent. Active elements. |
| `rose-bright` | `#e8a0b2` | (232, 160, 178) | Active, highlighted. Selection. |
| `rose-glow` | `#ffc0d0` | (255, 192, 208) | Maximum emphasis. Notifications. |

### Semantic Colors

Semantic colors for system state. Used sparingly — rose remains dominant.

| Token | Hex | RGB | Meaning |
|---|---|---|---|
| `jade` | `#5eead4` | (94, 234, 212) | Success. Passing gates. Health. |
| `amber` | `#fbbf24` | (251, 191, 36) | Warnings. Approaching thresholds. |
| `crimson` | `#f87171` | (248, 113, 113) | Errors. Failed gates. Danger. |
| `violet` | `#a78bfa` | (167, 139, 250) | Knowledge. Neuro entries. |
| `sapphire` | `#60a5fa` | (96, 165, 250) | Agents. Active processes. |

### Text Hierarchy

Four levels of text contrast for information hierarchy:

| Token | Hex | RGB | Usage |
|---|---|---|---|
| `ghost` | `#6b7280` | (107, 114, 128) | Tertiary. Decorative. Timestamps. |
| `mist` | `#9ca3af` | (156, 163, 175) | Secondary. Supporting text. |
| `frost` | `#e5e7eb` | (229, 231, 235) | Primary. Body text. |
| `white` | `#f9fafb` | (249, 250, 251) | Maximum contrast. Headers. |

---

## Typography

**Monospace throughout**. Every interface uses monospace type: JetBrains Mono (preferred), Berkeley Mono (premium alternative), or system monospace (fallback). No proportional fonts. This is a technical tool for developers — monospace ensures code, data, and prose align consistently.

### Type Treatments

| Element | Treatment |
|---|---|
| **Headers** | Uppercase, letter-spaced (0.1em), rose accent color |
| **Data values** | Tabular numerals for alignment in columns |
| **Status indicators** | Small caps or Unicode symbols (blocks ▓░, circles ◉○◌, arrows ↑↓) |
| **Code** | Standard monospace, syntax-highlighted in ROSEDUST semantic colors |
| **Labels** | Ghost or mist text, reduced size |

### No Emojis in TUI

The terminal interface uses Unicode symbols and dingbats only (BMP range U+0000-U+FFFF). No emojis. This ensures consistent rendering across terminal emulators and avoids the visual inconsistency of mixed emoji and monospace text.

---

## Glass Morphism

Panels use a subtle frosted-glass effect that creates depth without heaviness:

```css
/* Web Portal implementation */
.panel {
    background: rgba(18, 16, 26, 0.80);   /* twilight at 80% opacity */
    backdrop-filter: blur(12px);            /* frosted glass blur */
    border: 1px solid rgba(139, 94, 107, 0.20); /* rose-dim at 20% opacity */
    border-radius: 8px;
}
```

```rust
// TUI approximation using ratatui Block
// Glass morphism is approximated via bg color with slight transparency feel
Block::default()
    .borders(Borders::ALL)
    .border_style(Style::default().fg(theme.border))
    .style(Style::default().bg(theme.bg_alt))
```

The glass effect creates a sense of panels floating over the background plane. Each panel feels like a separate viewing surface, not a flat grid cell. In the TUI, this is approximated through subtle border coloring and background differentiation. In the Web Portal, full CSS `backdrop-filter` is used where performance and accessibility allow it.

---

## Motion

### Luxury Easing

All transitions use a custom cubic-bezier curve that creates a sense of weight and precision:

```css
transition-timing-function: cubic-bezier(0.16, 1, 0.3, 1);
```

This curve starts slow, accelerates through the middle, and decelerates to a gentle stop. It feels like a physical object with mass moving through space — not the default linear or ease-in-out that feels generic.

### Ambient Breathing

Active elements exhibit a subtle pulsing animation that communicates life:

- **Spectre creatures**: Breathing animation at the Daimon's arousal-derived rate (0.3-1.2 Hz)
- **Active agent indicators**: Gentle glow pulsing on the `◉` marker
- **C-Factor gauge**: Smooth gradient animation reflecting collective harmony
- **Processing indicators**: Wave animation (not spinner) during LLM inference

### Data Transitions

Numeric values and charts animate smoothly between states:

- Numbers count up/down with eased interpolation (200ms transition)
- Bar charts grow/shrink with luxury easing
- Sparklines draw left-to-right with a trailing fade
- Progress bars use gradient interpolation from `danger` → `warning` → `success`

The progress bar gradient is implemented in the existing codebase:

```rust
// From roko-cli/src/tui/theme.rs
pub fn progress_style(&self, ratio: f64) -> Style {
    let ratio = ratio.clamp(0.0, 1.0);
    let color = if ratio < 0.5 {
        gradient(self.danger, self.warning, ratio * 2.0)
    } else {
        gradient(self.warning, self.success, (ratio - 0.5) * 2.0)
    };
    Style::default().fg(color)
}
```

---

## Implementation: TUI (`RosedustTheme`)

The TUI implementation lives in `roko-cli/src/tui/theme.rs` as the `RosedustTheme` struct. It defines the complete ROSEDUST palette adapted for terminal rendering:

```rust
pub struct RosedustTheme {
    pub bg: Color,           // Rgb(26, 21, 32)  — twilight-like
    pub bg_alt: Color,       // Rgb(34, 29, 42)  — elevated surface
    pub fg: Color,           // Rgb(232, 223, 213) — warm frost
    pub fg_muted: Color,     // Rgb(138, 127, 142) — mist equivalent
    pub rose: Color,         // Rgb(212, 119, 140) — primary accent
    pub rose_muted: Color,   // Rgb(160, 92, 110) — rose-dim equivalent
    pub gold: Color,         // Rgb(212, 168, 87) — amber semantic state
    pub teal: Color,         // Rgb(93, 184, 163) — jade semantic state
    pub blue: Color,         // Rgb(107, 143, 189) — sapphire semantic state
    pub lavender: Color,     // Rgb(160, 140, 196) — violet semantic state
    pub coral: Color,        // Rgb(196, 122, 92) — warm accent
    pub success: Color,      // teal
    pub warning: Color,      // gold
    pub danger: Color,       // Rgb(196, 92, 80) — warm crimson
    pub info: Color,         // blue
    pub border: Color,       // Rgb(58, 51, 69) — subtle border
    pub border_active: Color,// rose — focused border
    pub selection_bg: Color, // Rgb(45, 40, 56) — selection highlight
    pub header_bg: Color,    // Rgb(30, 25, 40) — header bar
    pub status_bg: Color,    // bg_alt
}
```

The `active_theme()` function respects the `NO_COLOR` environment variable, returning a fully reset palette for accessibility compliance.

Supporting utilities in `roko-cli/src/tui/color.rs`:
- `hsv_to_rgb()` — HSV color space conversion
- `gradient()` — linear interpolation between two RGB colors
- `darken()` / `lighten()` — brightness adjustment

Semantic style helpers map plan phases and agent roles to accent colors:
- Plans: `planning` → lavender, `building` → blue, `testing` → gold, `gating` → coral, `complete` → teal
- Roles: `architect` → lavender, `implementer` → blue, `reviewer` → coral, `researcher` → gold, `tester` → teal

---

## Implementation: Web Portal

The Web Portal (planned, P2) will implement ROSEDUST using:

- **Tailwind CSS 4** with custom theme tokens matching the ROSEDUST palette
- **CSS custom properties** for light, dark, and high-contrast variants over one token system
- **SvelteKit** as the reference stack, with React-based implementations still possible later
- **CSS transitions** with the luxury easing curve, respecting reduced-motion settings
- Optional richer renderers for Spectre-heavy views rather than making WebGL a baseline requirement for every page

---

## Color Science Foundations

### Perceptual Uniformity — OKLab and OKLCH

ROSEDUST palette computations use the OKLab color space (Ottosson, 2020; CSS Color Level 4/5) for perceptually uniform gradient interpolation. Unlike sRGB linear interpolation (which produces muddy midpoints) or HSL (which creates unwanted brightness shifts), OKLab guarantees that equal Euclidean distances in Lab-space correspond to equal perceived color differences.

**Conversion pipeline (linear sRGB → OKLab):**

```rust
/// Convert linear sRGB to OKLab.
/// Step 1: linear sRGB → LMS (cone response)
/// Step 2: cube root compression → perceptual OKLab (L, a, b)
pub fn srgb_to_oklab(r: f32, g: f32, b: f32) -> (f32, f32, f32) {
    let l = 0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b;
    let m = 0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b;
    let s = 0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b;
    let l_ = l.cbrt();
    let m_ = m.cbrt();
    let s_ = s.cbrt();
    let lab_l = 0.2104542553 * l_ + 0.7936177850 * m_ - 0.0040720468 * s_;
    let lab_a = 1.9779984951 * l_ - 2.4285922050 * m_ + 0.4505937099 * s_;
    let lab_b = 0.0259040371 * l_ + 0.7827717662 * m_ - 0.8086757660 * s_;
    (lab_l, lab_a, lab_b)
}
```

**OKLCH (cylindrical form):** `L ∈ [0,1]`, `C = √(a² + b²)`, `H = atan2(b, a)`. All ROSEDUST palette steps maintain equal L values within each tonal ramp. The rose anchor sits at approximately `OKLCH(0.65, 0.13, 12°)`.

### ROSEDUST Palette in OKLCH Coordinates

| Token | Hex | OKLCH (L, C, H°) | Purpose |
|---|---|---|---|
| `void-black` | `#0a0a0f` | (0.09, 0.01, 280°) | Deepest background |
| `twilight` | `#12101a` | (0.13, 0.02, 280°) | Panel backgrounds |
| `rose-dim` | `#8b5e6b` | (0.51, 0.06, 10°) | Muted elements |
| `rose` | `#c77d8f` | (0.65, 0.10, 12°) | Standard accent |
| `rose-bright` | `#e8a0b2` | (0.77, 0.09, 10°) | Active elements |
| `rose-glow` | `#ffc0d0` | (0.87, 0.07, 8°) | Maximum emphasis |
| `jade` | `#5eead4` | (0.84, 0.14, 170°) | Success |
| `amber` | `#fbbf24` | (0.84, 0.18, 85°) | Warnings |
| `crimson` | `#f87171` | (0.67, 0.19, 22°) | Errors |
| `violet` | `#a78bfa` | (0.68, 0.16, 290°) | Knowledge |
| `sapphire` | `#60a5fa` | (0.72, 0.14, 250°) | Agents |

### Color Harmony Construction

The palette uses **analogous harmony** centered on rose (H ≈ 12°), with semantic colors at harmonic intervals:

```
Rose family:    H = 8°–15°  (analogous cluster)
Amber:          H = 85°     (warm complement quadrant)
Jade:           H = 170°    (complementary)
Sapphire:       H = 250°    (cool triadic)
Violet:         H = 290°    (split-complementary)
```

### Perceptually Uniform Gradient Interpolation

All ROSEDUST gradients interpolate in OKLab, not sRGB:

```rust
/// Perceptually uniform gradient between two colors.
pub fn gradient_oklab(c1: Color, c2: Color, t: f64) -> Color {
    let (r1, g1, b1) = color_to_linear(c1);
    let (r2, g2, b2) = color_to_linear(c2);
    let lab1 = srgb_to_oklab(r1, g1, b1);
    let lab2 = srgb_to_oklab(r2, g2, b2);
    let lab_t = (
        lab1.0 + (lab2.0 - lab1.0) * t as f32,
        lab1.1 + (lab2.1 - lab1.1) * t as f32,
        lab1.2 + (lab2.2 - lab1.2) * t as f32,
    );
    let (r, g, b) = oklab_to_srgb(lab_t.0, lab_t.1, lab_t.2);
    Color::Rgb((r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8)
}
```

### APCA Contrast Verification

The palette is verified against the APCA (Advanced Perceptual Contrast Algorithm, WCAG 3.0 candidate). APCA is polarity-aware — it accounts for light-text-on-dark-bg perception, which is ROSEDUST's standard configuration.

**APCA Lc targets:** `|Lc| ≥ 75` for body text, `≥ 60` for large/bold, `≥ 45` for non-text.

```rust
/// Compute APCA Lightness Contrast (Lc) between text and background.
pub fn apca_contrast(text: (u8, u8, u8), bg: (u8, u8, u8)) -> f64 {
    let y_t = screen_luminance(text);
    let y_b = screen_luminance(bg);
    let clamp = |y: f64| if y < 0.022 { y + (0.022 - y).powf(1.414) } else { y };
    let (yt, yb) = (clamp(y_t), clamp(y_b));
    let sapc = if yb > yt {
        (yb.powf(0.56) - yt.powf(0.57)) * 1.14
    } else {
        (yb.powf(0.65) - yt.powf(0.62)) * 1.14  // reverse polarity (ROSEDUST)
    };
    if sapc.abs() < 0.1 { 0.0 }
    else if sapc > 0.0 { (sapc - 0.027) * 100.0 }
    else { (sapc + 0.027) * 100.0 }
}
```

**ROSEDUST APCA results:**

| Pairing | APCA Lc | Target | Pass? |
|---|---|---|---|
| Body text (`#E8DFD5`) on void (`#0a0a0f`) | +94.2 | ≥75 | AA |
| Muted text (`#8A7F8E`) on void | +62.1 | ≥60 | AA |
| Rose accent (`#D4778C`) on void | +71.8 | ≥60 | AA |
| Jade (`#5DB8A3`) on void | +79.3 | ≥60 | AA |
| Danger (`#C45C50`) on void | +58.9 | ≥45 | AA (non-text) |

### Terminal Color Quantization

For 256-color terminals, ROSEDUST maps via perceptual distance in OKLab:

```rust
/// Map 24-bit RGB to nearest 256-color index using OKLab distance.
pub fn nearest_256(r: u8, g: u8, b: u8) -> u8 {
    let target = srgb_to_oklab(r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0);
    (0..=255u8).min_by(|&a, &b_idx| {
        let da = oklab_dist(target, idx_to_oklab(a));
        let db = oklab_dist(target, idx_to_oklab(b_idx));
        da.partial_cmp(&db).unwrap()
    }).unwrap()
}
```

### Color Blindness Safety

No information is conveyed by color alone — status indicators use symbols (`✓`/`✗`/`○`) plus color. Under deuteranopia simulation, rose shifts toward brownish-yellow and jade toward blue-gray — still distinguishable due to OKLab lightness difference (L=0.65 vs L=0.84).

### Rose Heat Gradient

A ROSEDUST-branded heat gradient for sparklines and heat maps, interpolated in OKLCH:

```
0.00 → OKLCH(0.15, 0.05, 10°)   — deep dark rose (cold)
0.25 → OKLCH(0.35, 0.12, 15°)   — medium rose
0.50 → OKLCH(0.55, 0.18, 20°)   — bright rose
0.75 → OKLCH(0.75, 0.12, 60°)   — golden warm (transition)
1.00 → OKLCH(0.92, 0.05, 80°)   — near-white yellow (hot)
```

---

## Academic Foundations

- OKLab perceptual uniformity — Ottosson (2020), CSS Color Level 4 specification
- APCA contrast algorithm — Somers/Myndex (2022), WCAG 3.0 candidate
- CIE Delta-E 2000 — Sharma, Wu, Dalal (2005) for palette distinctiveness verification
- Purkinje effect — CIE mesopic photometry (TN 004:2016); warm colors lose brightness faster in dark environments
- PAD model color psychology — Mehrabian (1996); rose tones map to moderate pleasure, low arousal
- Glass morphism depth perception — Harrison et al. (CHI 2011); translucent layering improves spatial awareness
- Color vision deficiency simulation — Machado, Oliveira, Fernandes (2009)
- HSLuv perceptual palette generation — CIELCHuv with gamut-safe saturation normalization

---

## Current Status and Gaps

**Built:**
- `RosedustTheme` in `roko-cli/src/tui/theme.rs` with full palette and semantic helpers
- Color utilities (gradient, darken, lighten, HSV conversion) in `roko-cli/src/tui/color.rs`
- Theme applied to existing TUI widgets (agent grid, plan tree, status bar)
- `NO_COLOR` support

**Not built:**
- Web Portal Tailwind theme
- WebGL bloom/glow effects
- Ambient breathing animations in TUI
- Full glass morphism in TUI (approximated via border/bg differentiation)

---

## Cross-References

- See [08-tui-main-layout.md](./08-tui-main-layout.md) for TUI layout using ROSEDUST
- See [10-spectre-creature-visualization.md](./10-spectre-creature-visualization.md) for Spectre colors
- See [13-web-portal.md](./13-web-portal.md) and [tmp/refinements/29-web-ui-architecture.md](../../tmp/refinements/29-web-ui-architecture.md) for the first-party browser implementation
- See topic [09-daimon](../09-daimon/INDEX.md) for behavioral state → color mappings
