# MailSlate — Design System

| Field | Value |
| --- | --- |
| Version | 1.0 |
| Date | 2026-10-16 |
| Status | Approved for implementation |
| Source | [DESIGN-cal.md](./DESIGN-cal.md) — Cal.com design system analysis (version: alpha) |
| Related | [PRD](./PRD.md) · [ARCHITECTURE](./ARCHITECTURE.md) · [USER_FLOW](./USER_FLOW.md) · [APP_ICON](./APP_ICON.md) |

This document adapts the Cal.com design tokens (captured in `DESIGN-cal.md`) from a **marketing
website system** into the **in-app chrome** of a desktop mail client: sidebar, conversation list,
reader, composer, search, settings. Light and dark themes are both fully specified; the app
follows the system theme by default.

---

## 1. Design principles

1. **Monochrome action layer.** CTAs are near-black (light) / near-white (dark). Accent colors
   never paint surfaces. Cal.com's rule survives the adaptation.
2. **Hairlines, not shadows.** Structure comes from 1px dividers and surface alternation. Shadows
   exist only on floating layers (menus, popovers, dialogs).
3. **Type does the talking.** Cal Sans (substituted) for display moments — onboarding, empty
   states only. Inter for everything else. Negative tracking only on display sizes.
4. **Fixed comfortable density.** 44px list rows, 32px sidebar items, 40px inputs, 24px reader
   padding. Compact mode is v2.
5. **Color is semantic, never decorative.** Accent blue = focus ring, links, selection.
   Orange = starred. Pastels = tags/identity. Success/warning/error for state only.
6. **Scarcity of the dark surface (light theme).** Cal.com reserves dark surfaces for the footer
   and featured tier. In-app, the dark palette is a *full theme inversion* (user-approved
   extension), and within the light theme no chrome surface is ever dark.
7. **Calm motion.** ~150ms color/opacity transitions. No decorative animation.

---

## 2. Adaptation strategy

| Cal.com (marketing) | MailSlate (app) |
| --- | --- |
| White canvas pages, alternating light-gray bands | Canvas + `surface-soft` chrome (sidebar), `canvas` content, hairline dividers |
| `button-primary` black CTA | Primary buttons (Compose, Send, dialogs) — same tokens |
| `nav-pill-group` pill-in-pill switcher | Segmented filter control (list filters, settings tabs) with white active pill on `surface-soft` track |
| `top-nav` 64px | App toolbar (~56px) — same type/color rules, slightly tighter |
| `feature-card` gray card | Tag chips, sync status cards, settings cards where a container is needed |
| `hero-app-mockup-card` | Onboarding/empty-state media panel (type-led; no illustration overload) |
| `pricing-tier-card-featured` (only dark card) | Dark theme extension base: `surface-dark` / `surface-dark-elevated` reused as dark canvas layers |
| `footer` (dark on light) | Settings → About / account footer panels *inside dark theme* only |
| Testimonial / pricing / cta-band / rating-stars | **N/A** — marketing-only components, not adapted |
| `badge-pill` pastels | Tag chips + account identity colors |
| Radius ≤ 16px, weight ≤ 600, no hover beyond press | Carried over verbatim |

---

## 3. Color system

### 3.1 Light theme tokens

| Token | Value | Usage |
| --- | --- | --- |
| `canvas` | `#ffffff` | Reader background, list background, content surfaces |
| `surface-soft` | `#f8f9fa` | Sidebar background, toolbar background, search track, hover fill |
| `surface-card` | `#f5f5f5` | Settings cards, banner track, skeleton base, avatar fill base |
| `surface-strong` | `#e5e7eb` | Disabled fills, stronger section fills (rare) |
| `hairline` | `#e5e7eb` | 1px dividers: pane borders, row separators, input borders |
| `hairline-soft` | `#f3f4f6` | Divider inside cards/settings groups |
| `ink` | `#111111` | Primary text, icons active, primary button bg |
| `body` | `#374151` | Secondary running text, email body text |
| `muted` | `#6b7280` | Timestamps, metadata, idle icons |
| `muted-soft` | `#898989` | Captions, fine print, placeholders |
| `on-primary` | `#ffffff` | Text on primary buttons |
| `primary` | `#111111` | Primary button bg |
| `primary-active` | `#242424` | Primary button pressed |
| `primary-disabled` | `#e5e7eb` (+ `muted` text) | Disabled buttons |
| `accent` | `#3b82f6` | Focus rings, text links, selection tint (only these) |
| `accent-tint` | `rgba(59,130,246,0.12)` | Selected row/thread background tint |
| `star` | `#fb923c` | Starred state (badge-orange repurposed) |
| `success` | `#10b981` | Sync OK, send success toast icon |
| `warning` | `#f59e0b` | Offline banner icon, warning callouts |
| `error` | `#ef4444` | Validation, failed send, destructive actions |
| `badge-orange/pink/violet/emerald` | `#fb923c` `#ec4899` `#8b5cf6` `#34d399` | Tag chips + account identity dots |

### 3.2 Dark theme tokens

| Token | Value | Notes |
| --- | --- | --- |
| `canvas` | `#101010` (`surface-dark`) | App floor |
| `surface-soft` | `#151515` | Sidebar/toolbar — one step above canvas |
| `surface-card` | `#1a1a1a` (`surface-dark-elevated`) | Cards, hover fill |
| `surface-strong` | `#262626` | Disabled fills |
| `hairline` | `rgba(255,255,255,0.10)` | ~10% white — visible on `#101010` |
| `hairline-soft` | `rgba(255,255,255,0.06)` | In-card dividers |
| `ink` | `#fafafa` | Primary text/icons |
| `body` | `#d4d4d8` | Running text |
| `muted` | `#a1a1aa` (`on-dark-soft`) | Timestamps, metadata |
| `muted-soft` | `#71717a` | Captions, placeholders |
| `on-primary` | `#111111` | Text on the (white) primary button |
| `primary` | `#fafafa` | Monochrome action layer inverts: white button, near-black text |
| `primary-active` | `#e5e7eb` | Pressed |
| `primary-disabled` | `#262626` (+ `muted-soft` text) | Disabled |
| `accent` | `#3b82f6` | Unchanged — focus/links/selection |
| `accent-tint` | `rgba(59,130,246,0.20)` | Selection tint (slightly stronger on dark) |
| `star` | `#fb923c` | Unchanged |
| `success` / `warning` / `error` | unchanged | Semantic colors stay identical across themes |
| Badge pastels | unchanged | Identity is theme-independent |

### 3.3 Usage rules

- **Contrast floor:** all text ≥ 4.5:1 against its surface (verified pairs listed in §11).
- **No accent on surfaces.** `accent` appears only as 2px focus ring, link text, 1px selection
  border or 12–20% tint fill. Never a button background, never a pane.
- **State never color-only.** Unread = weight + dot; starred = filled glyph + color; error = icon
  + text; offline = icon + banner copy.
- **Tag chip coloring:** Gmail label colors map to the pastel set by nearest hue (orange/pink/
  violet/emerald); unmapped labels default to `surface-card` + `ink` text. Local tags cycle the
  four pastels deterministically. Pastel chips use `ink` text at 90% opacity on light; in dark
  theme chips use 18% pastel tint + pastel-tinted text.

---

## 4. Typography

### 4.1 Families

- **Base:** Inter — all UI, body, buttons, nav. Bundled variable font (400/500/600), no network
  fetch.
- **Display substitute:** Cal Sans is not publicly licensable. Replacement: **Inter 600 with
  −0.04em tracking** (per `DESIGN-cal.md` substitution note). Optional polish: Manrope 700.
- **Mono:** JetBrains Mono — only for the About screen version/protocol strings and log excerpts.

### 4.2 App type scale

| Token | Size / Weight / Line / Tracking | Usage |
| --- | --- | --- |
| `display-sm` | 28 / 600 / 1.2 / −0.5px | Onboarding headline, empty-state headline |
| `title-lg` | 22 / 600 / 1.3 / −0.3px | Settings page title, account section heads |
| `title-md` | 18 / 600 / 1.4 / 0 | Dialog titles, settings group headers |
| `title-sm` | 16 / 600 / 1.4 / 0 | Reader subject line, composer subject input |
| `body-md` | 16 / 400 / 1.5 / 0 | **Email body rendering**, reader quoting, settings descriptions |
| `body-sm` | 14 / 400 / 1.5 / 0 | List row snippet, general UI text, search results |
| `caption` | 13 / 500 / 1.4 / 0 | Timestamps, tag chips, counts badges, banner copy |
| `button` | 14 / 600 / 1.0 / 0 | All buttons |
| `nav-link` | 14 / 500 / 1.4 / 0 | Sidebar items, toolbar labels, menu items |
| `code` | 14 / 400 / 1.5 / 0 (mono) | About screen, diagnostics |

List-row composition: sender + subject in `body-sm` weight 600 when unread, 500 when read;
snippet `body-sm` 400 `muted`; time `caption` `muted`; thread count `caption` in a `surface-card`
pill. Reader header: subject `title-sm`, metadata `caption` `muted`.

Negative tracking appears **only** on `display-sm` and above (empty states, onboarding). Never on
body text. Weight never exceeds 600 (Cal.com rule carried over).

---

## 5. Spacing, sizing, layout

- Base unit 4px. Tokens: `xxs` 4 · `xs` 8 · `sm` 12 · `md` 16 · `lg` 24 · `xl` 32 · `xxl` 48.
- **Panes:** sidebar 240px fixed; conversation list ~380px (fixed v1, resizable v2); reader fills.
- **Sidebar:** item height 32px, radius 8, padding 0 8px; group headers `caption` uppercase 11px
  with 12px top margin; account dot 8px circle + 8px gap.
- **List:** row height 44px (single line snippet), padding 8px 12px; avatar 32px in list rows;
  row gap 0 (hairlines separate); selected row = `accent-tint` + 2px left `accent` inset bar.
- **Reader:** padding 24px; message header block separated by `hairline-soft`; quote chip inline.
- **Composer:** padding 24px; field rows 40px; body area = borderless, `body-md`.
- **Toolbar:** height 56px, `surface-soft`, hairline bottom.
- **Settings:** max content width 720px, group spacing 32px, row height 40px.
- **Dialogs:** width per content (min 360, max 560), padding 24, radius 12.
- **Minimum window:** 1024 × 640.

---

## 6. Shape & elevation

| Token | Value | Usage |
| --- | --- | --- |
| `r-xs` | 4px | Count pills (rare) |
| `r-sm` | 6px | Menu items, small inline buttons |
| `r-md` | 8px | Buttons, inputs, sidebar items, tabs |
| `r-lg` | 12px | Cards, panes' inner cards, dialogs, popovers, banners |
| `r-xl` | 16px | Onboarding/empty-state media panel (max allowed) |
| `r-pill` | 9999px | Tag chips, filter chips, segmented control track |
| `r-full` | 9999px | Avatars, account dots, icon buttons |

Nothing above 16px except pills/full (Cal.com rule).

**Elevation:** chrome is flat. Shadows only on floating layers:

- Light: `0 1px 2px rgba(0,0,0,0.05)`, `0 4px 12px rgba(0,0,0,0.08)` (popover/menu/dialog).
- Dark: `0 8px 24px rgba(0,0,0,0.5)` single-layer.
- Segmented control active pill: `0 1px 2px rgba(0,0,0,0.05)` (the pill-in-pill signature).

---

## 7. Component catalog

States are listed as default / hover / pressed / disabled / focus where applicable. Focus = 2px
`accent` ring, offset 1px (all interactive elements).

### 7.1 Buttons

| Component | Spec |
| --- | --- |
| `button-primary` | h40, pad 12×20, r8, bg `primary`, text `on-primary` `button`; hover: bg `primary-active` (dark: `primary-active`); pressed: 1px scale-down 0.98; disabled: `primary-disabled` + `muted` |
| `button-secondary` | h40, r8, bg `canvas`, text `ink`, 1px `hairline` border; hover: `surface-soft` |
| `button-ghost` | h32/40, r8, transparent, text `ink`; hover: `surface-soft`; used for toolbar/dialog tertiary (Cancel) |
| `button-destructive` | as secondary with `error` text + `error` 1px border; hover: 8% `error` tint fill |
| `button-icon-circular` | 36px circle, hairline border, icon `muted`→`ink` on hover, hover fill `surface-soft` |
| `button-icon-ghost` | 28px square r-sm transparent, icon `muted`; hover `surface-soft` + `ink` (row hover actions) |

### 7.2 Inputs

| Component | Spec |
| --- | --- |
| `text-input` | h40, r8, pad 10×14, bg `canvas`, 1px `hairline`; focus: border `ink` (Cal carry-over) + ring; error: border `error` + helper text `caption` `error` |
| `search-input` | h32, r-pill in toolbar; bg `surface-soft`, no border; leading 16px search icon `muted`; focus: `canvas` bg + ring |
| `chip-input` | Recipient fields: chips (r-pill, `surface-card`) h24 + inline text; field container = `text-input` styling with 6px padding |
| `compose-body` | Borderless, `body-md`, min-height fills pane; placeholder `muted-soft` |
| `dropdown` | h32 (menus), r-sm items, hover `surface-soft`; menu container r-lg + shadow, pad 4 |

### 7.3 Navigation & structure

| Component | Spec |
| --- | --- |
| `sidebar` | bg `surface-soft`, right hairline; 240px |
| `sidebar-item` | h32, r8, pad 0 8px, `nav-link`; icon 16 `muted`; hover `surface-card`; selected `accent-tint` + `ink` text; count badge `caption` `muted` right-aligned |
| `account-group-header` | dot 8px (pastel) + account name `caption` 600, uppercase; row h28; collapse chevron 12px |
| `toolbar` | h56 `surface-soft`; contains sync-state cluster, search, refresh, `.` menu |
| `segmented-control` | nav-pill-group adaptation: track r-pill bg `surface-soft` pad 4; segment h28 pad 0 12 `nav-link`; active segment = `canvas` pill r-pill + shadow (signature pill-in-pill) |
| `context-menu` | Right-click menu: r-lg, shadow, items h32 r-sm `nav-link`; destructive items `error` text |
| `popover` | r-lg shadow, pad 12–16 |

### 7.4 Mail content

| Component | Spec |
| --- | --- |
| `list-row` | h44; sender `body-sm` w600 (unread) / w500 (read); subject same weight scale; snippet `muted`; time `caption` `muted` right; unread dot 8px `ink`; star icon 16 `star` filled / `muted-soft` outline; attachment glyph 14 `muted`; hover `surface-soft`; selected `accent-tint` + left bar; multi-select checkbox 16px appears on hover |
| `thread-view` | Messages stacked; collapsed message = 2-line header (avatar 32, sender, date, snippet, chevron); expanded = full body `body-md` + attachment chips; unread accent bar 2px left `accent` |
| `quote-chip` | Inline pill `surface-card` `caption` `muted` with chevron; expands quoted block (`body-sm`, `muted`, left hairline) |
| `attachment-chip` | r-md `surface-card` pad 6×10: type icon 16 + name `caption` + size `muted-soft`; hover `surface-strong` |
| `reader-header` | Subject `title-sm`; sender row avatar 36 + name `body-sm` 600 + address `caption` `muted`; actions (reply, star, more) at right as `button-icon-ghost` |
| `tag-chip` | r-pill `caption`, pad 4×12; colors per §3.3; removable (× on hover) in edit contexts |
| `account-dot` | 8px circle, pastel fill — identity in sidebar/list/settings |

### 7.5 Feedback & overlays

| Component | Spec |
| --- | --- |
| `banner` | Full-list width, h36, r0 (flush) or r-lg when floating; variants: offline `warning` 8% tint + icon; error `error` 8% tint; info `surface-card` |
| `toast` | Floating bottom-left above list: r-lg, `surface-dark` bg + white text in light theme (scarce dark surface — allowed for transient layer), shadow; action link `accent`; auto-dismiss 6s |
| `dialog` | r-lg, pad 24; title `title-md`; body `body-sm`; actions right-aligned (Cancel ghost + primary/destructive) |
| `sync-progress` | Account row: 3px indeterminate bar `accent` under account header, or inline spinner 12px `muted` |
| `skeleton` | `surface-card` blocks r-sm; rows mimic list-row silhouette; no shimmer (static, calm) |
| `empty-state` | Centered: display-sm headline, `body-sm` `muted` line, one primary button; used in list/reader/search |
| `count-badge` | r-pill min 20px h, pad 0 6, `caption`, bg `surface-card`; unread counts in sidebar |
| `status-dot` | 8px: `success` synced, `warning` syncing/offline, `error` failed |

### 7.6 Unadapted Cal.com components (N/A)

`top-nav` (marketing nav), `hero-band`, `pricing-tier-card`, `testimonial-card`, `cta-band-light`,
`footer`, `rating-stars`, `product-mockup-card`, `feature-icon-card` — marketing-only; not
implemented in the app. Their token *rules* still inform §2's adaptation table.

---

## 8. Iconography

- **Set:** Material Symbols Rounded (outline), via `material_symbols_icons`; fallback to bundled
  Material Icons if the package lags a Flutter release. Outline style matches the hairline system.
- **Sizes:** 16 (inline: star, attachment glyph, chips), 20 (toolbar, sidebar), 24 (reader
  actions, dialogs). Stroke-weight feel stays light; never emoji, never filled except state
  communicates (filled star when starred, filled dot for unread).
- **Colors:** idle `muted`; hover/active `ink`; selected `ink`; destructive `error`; star `star`.
- **Geometry:** icons align to the 4px grid; icon-only buttons always `button-icon-circular` (36)
  or `button-icon-ghost` (28) — never bare touch targets.

---

## 9. Motion

| Interaction | Motion |
| --- | --- |
| Hover / press / selection | 150ms `easeOut` color/background |
| Banner / toast enter | 200ms fade + 8px slide |
| Pane route (composer) | 200ms slide x ±8px + fade |
| Popover / menu open | 120ms fade + 4px scale from 0.98 |
| Skeleton → content | 150ms crossfade |
| Reduced motion (OS setting) | All durations → 0ms; fades → instant |

No decorative animation, no looping UI (Cal.com calm). The only indeterminate motion is the sync
progress bar.

---

## 10. Dark theme derivation rules

1. Floor inverts to `surface-dark` (#101010); every layer shifts one step up the same way the
   light theme shifts down (`#101010 → #151515 → #1a1a1a → #262626`).
2. Primary action inverts to near-white (`#fafafa`) with `ink` text — keeps the monochrome
   action layer rule.
3. Semantic colors (accent/star/success/warning/error, badge pastels) are **identical** in both
   themes; only surfaces, hairlines, and text tokens change.
4. Hairlines become white alpha (10% / 6%) — never solid grays on dark.
5. Product polish toasts may keep the dark surface in light theme (transient overlay, not chrome).
6. Theme follows system by default; Settings → Appearance overrides (System / Light / Dark).

---

## 11. Accessibility

- Contrast pairs verified: `ink`/`canvas` 17.9:1 · `body`/`canvas` 9.6:1 · `muted`/`canvas` 4.8:1 ·
  `on-primary`/`primary` 17.9:1 · dark `ink`(#fafafa)/`canvas`(#101010) 16.5:1 · dark
  `muted`(#a1a1aa)/`canvas` 6.9:1 · `accent`/`canvas` 3.7:1 (non-text use only: rings, bars, tints).
- Focus ring 2px `accent`, offset 1px, on every interactive element; focus order = visual order.
- Never color-only state (see §3.3).
- Screen-reader labels on icon buttons; list rows read sender/subject/time/flags (USER_FLOW §13).
- Reduced motion respected (§9). Text-scaling: layouts tolerate 100–125% before truncation.

---

## 12. Do & Don't (app edition)

**Do**

- Keep the action layer monochrome; use pastels only for tags/identity.
- Use hairlines and surface alternation for structure; shadows only on floats.
- Keep Inter 400 body / 600 titles; negative tracking only on display.
- Use one primary button per surface (compose empty state, dialog, onboarding step).
- Alternate surfaces as in Cal pacing: sidebar `surface-soft` → list `canvas` → reader `canvas`
  with hairline structure.

**Don't**

- Never place `accent`, `star`, or pastels on button backgrounds.
- Never exceed radius 16px on cards; never exceed weight 600.
- No hover styling beyond what §7 encodes; pressed states are subtle (darken/scale 0.98).
- No dark chrome surfaces inside light theme (except transient toast).
- No gradients, glassmorphism, neumorphism, bevels. Flat + hairline only.

---

## 13. Cross-links

- Raw token source and rationale: [DESIGN-cal.md](./DESIGN-cal.md).
- Flow behavior, keyboard map, state matrix: [USER_FLOW](./USER_FLOW.md).
- Theme plumbing (`ThemeData` light/dark, token extension class): [ARCHITECTURE](./ARCHITECTURE.md) §4.
- App icon, tray icon, notification icon: [APP_ICON](./APP_ICON.md).
