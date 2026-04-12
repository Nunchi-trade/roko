# ROSEDUST Design Language

> The visual identity of Roko: rose tones on deep violet-black, glass morphism, luxury motion, dark-only — shared across TUI, Web Portal, and Spectre visualization.


> **Implementation**: Scaffold

**Topic**: [12-interfaces](./INDEX.md)
**Prerequisites**: None
**Key sources**: `refactoring-prd/06-interfaces.md` §3, `roko-cli/src/tui/theme.rs`, `roko-cli/src/tui/color.rs`, `bardo-backup/prd/shared/branding.md` §5.2

---

## Abstract

ROSEDUST is Roko's design language — a comprehensive visual system that unifies the appearance of every interface surface: the Terminal UI (ratatui), the Web Portal (React/Next.js), Spectre creature visualizations, and CLI output. The name evokes the palette's essential character: rose light on void-black, as if viewing the system through a faintly glowing, dusty lens.

ROSEDUST is **dark-only**. There is no light mode. The design language is inherently built around contrast between deep backgrounds and glowing accents. The rose palette dominates, accounting for approximately 80% of visible accent color on any screen. Signal colors (jade, amber, crimson, violet, sapphire) provide semantic differentiation without breaking the rose-dominant aesthetic.

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

### Signal Colors

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
| **Code** | Standard monospace, syntax-highlighted in ROSEDUST signal colors |
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

The glass effect creates a sense of panels floating over the void-black background. Each panel feels like a separate viewing surface, not a flat grid cell. In the TUI, this is approximated through subtle border coloring and background differentiation. In the Web Portal, full CSS `backdrop-filter` is used.

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
    pub gold: Color,         // Rgb(212, 168, 87) — amber signal
    pub teal: Color,         // Rgb(93, 184, 163) — jade signal
    pub blue: Color,         // Rgb(107, 143, 189) — sapphire signal
    pub lavender: Color,     // Rgb(160, 140, 196) — violet signal
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
- **CSS custom properties** for dynamic theming (though always dark)
- **WebGL** via react-three-fiber for Spectre rendering with full glow/bloom effects
- **Framer Motion** or CSS transitions with the luxury easing curve

---

## Academic Foundations

- The color psychology of rose/pink tones in UI design draws on research in affective computing and the PAD (Pleasure-Arousal-Dominance) model by Mehrabian (1996) — rose tones map to moderate pleasure, low arousal, creating a calm but engaged viewing state
- The glass morphism approach is informed by depth perception research in HCI — translucent layering provides better spatial awareness than flat layouts (Harrison et al., CHI 2011)

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

## Cross-references

- See [08-tui-main-layout.md](./08-tui-main-layout.md) for TUI layout using ROSEDUST
- See [10-spectre-creature-visualization.md](./10-spectre-creature-visualization.md) for Spectre colors
- See [13-web-portal.md](./13-web-portal.md) for Web Portal implementation
- See topic [09-daimon](../09-daimon/INDEX.md) for behavioral state → color mappings
