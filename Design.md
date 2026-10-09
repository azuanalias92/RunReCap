---
name: Apple
colors:
  primary: "#0071e3"
  secondary: "#1d1d1f"
  background: "#ffffff"
  surface: "#f5f5f7"
  foreground: "#1d1d1f"
  border: "#d2d2d7"
  accent: "#86868b"
  success: "#34c759"
  info: "#0066cc"
  warning: "#ff9500"
  danger: "#ff3b30"
colors-dark:
  primary: "#2997ff"
  secondary: "#f5f5f7"
  background: "#000000"
  surface: "#272729"
  foreground: "#f5f5f7"
  border: "#333336"
  accent: "#98989d"
  success: "#30d158"
  info: "#2997ff"
  warning: "#ffd60a"
  danger: "#ff453a"
typography:
  display-lg:
    fontFamily: '"Inter", -apple-system, BlinkMacSystemFont, "SF Pro Display", "Helvetica Neue", Helvetica, Arial, sans-serif'
    fontSize: 56px
    fontWeight: 600
  heading-md:
    fontFamily: '"Inter", -apple-system, BlinkMacSystemFont, "SF Pro Display", "Helvetica Neue", Helvetica, Arial, sans-serif'
    fontSize: 40px
    fontWeight: 600
  body-md:
    fontFamily: '"Inter", -apple-system, BlinkMacSystemFont, "SF Pro Text", "Helvetica Neue", Helvetica, Arial, sans-serif'
    fontSize: 17px
    fontWeight: 400
  label-md:
    fontFamily: '"Inter", -apple-system, BlinkMacSystemFont, "SF Pro Text", "Helvetica Neue", Helvetica, Arial, sans-serif'
    fontSize: 14px
    fontWeight: 400
  caption-sm:
    fontFamily: '"Inter", -apple-system, BlinkMacSystemFont, "SF Pro Text", "Helvetica Neue", Helvetica, Arial, sans-serif'
    fontSize: 12px
    fontWeight: 400
spacing:
  xs: 2px
  sm: 4px
  md: 8px
  lg: 16px
  xl: 24px
rounded:
  sm: 5px
  md: 8px
  lg: 11px
  xl: 12px
  full: 9999px
---

Apple's color story is starkly binary. Product sections alternate between pure black (#000000) backgrounds with white text and light gray (#f5f5f7) backgrounds with near-black text (#1d1d1f). This creates cinematic pacing — dark sections feel immersive, light sections feel open.

### The Single Accent Principle

**Primary — Apple Blue (#0071e3)**: The ONLY chromatic accent in the entire interface. Reserved exclusively for interactive elements: links, buttons, and focus states.

### Surface Hierarchy

| Level | Light Mode | Dark Mode | Use |
|-------|-----------|-----------|-----|
| Background | #ffffff | #000000 | Page canvas |
| Elevated | #f5f5f7 | #1d1d1f | Cards in context |
| Glass | rgba(255,255,255,0.8) + blur | rgba(0,0,0,0.8) + blur | Navigation, modals |
| Interactive | #0071e3 | #2997ff | CTAs, links, focus |

---

## Typography

### Font Stack

**Inter** — Engineered for digital interfaces with variable fonts and optical sizing. At Apple scale, it channels SF Pro's DNA while remaining openly available.

### Type Scale

| Role | Size | Weight | Usage |
|------|------|--------|-------|
| Display Hero | 56px | 600 | Hero headlines |
| Section Heading | 40px | 600 | Section titles |
| Tile Heading | 28px | 400 | Card headers |
| Card Title | 21px | 700 | Product card titles |
| Body | 17px | 400 | Content text |
| Caption | 14px | 400 | Secondary info |
| Micro | 12px | 400 | Fine print |

---

## Layout & Spacing

The spacing system uses a 2px base unit — tighter than typical scales, reflecting Apple's precision aesthetic:

| Token | Value | Usage |
|-------|-------|-------|
| xs | 2px | Micro spacing |
| sm | 4px | Tight gaps |
| md | 8px | Default spacing, button padding |
| lg | 16px | Card padding, section spacing |
| xl | 24px | Large margins |

---

## Elevation & Depth

Apple uses shadow extremely sparingly:

| Level | Treatment | Usage |
|-------|-----------|-------|
| Flat | No shadow | Content sections, text |
| Card | `3px 5px 30px rgba(0,0,0,0.22)` | Product cards, elevated elements |
| Nav Glass | `backdrop-filter: blur(20px)` on rgba(0,0,0,0.8) | Floating navigation |
| Focus Ring | 2px solid #0071e3 | Keyboard focus |

### The Border Exception

Apple almost never uses visible borders on cards or containers. Elevation comes from background color contrast.

---

## Shapes

| Token | Value | Usage |
|-------|-------|-------|
| sm | 5px | Small elements |
| md | 8px | Buttons, inputs, cards |
| lg | 11px | Filter buttons |
| xl | 12px | Large elements |
| pill | 980px | Pill CTAs, "Learn more" links |
| full | 9999px | Badges |

---

## Components

### Buttons & Interaction

**Primary CTA**: Apple Blue (#0071e3), white text, 8px rounded. Hover: opacity 0.88. **Secondary**: Near Black (#1d1d1f), white text. **Pill Links**: Capsule shape (980px radius), colored border, transparent background.

### Inputs & Selection

**Text Inputs**: White background, #d2d2d7 border. Focus: Apple Blue border with blue glow ring. **Switches**: iOS-style toggle, 51x31px track, green when active (#34c759).

### Chips & Selection Controls

**Checkboxes**: Custom rounded, Apple Blue fill when checked. **Radio**: System-styled with Apple Blue accent.

### Data & Containers

**Cards**: #f5f5f7 background, 8px corners, no border. On hover: soft shadow appears. **Progress Bars**: 6px height, pill track, Apple Blue fill.

### Feedback Components

**Alerts**: 4px left-bordered accent stripe. **Toasts**: Glass-morphism panel with blur. **Badges**: Pill-shaped (9999px), small, uppercase.

---

## Do's and Don'ts

### Do

- ✅ Use Apple Blue ONLY for interactive elements — singular accent
- ✅ Apply negative letter-spacing at ALL sizes for precise feel
- ✅ Alternate between black and light gray section backgrounds
- ✅ Use 980px border-radius for pill CTAs
- ✅ Use translucent glass navigation — rgba(0,0,0,0.8) + blur

### Don't

- ❌ Don't introduce additional accent colors
- ❌ Don't use heavy or multi-layered shadows
- ❌ Don't use borders on cards or containers
- ❌ Don't use weight 800 or 900
- ❌ Don't add textures, patterns, or gradients to backgrounds
- ❌ Don't use rounded corners larger than 12px on rectangular elements
