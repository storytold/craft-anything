# Design system

The desktop app draws every screen from one scale and one set of components, so sizes, buttons
and gaps are the same everywhere. The scale lives in `crates/ui-egui/src/theme.rs`; the
components live in `crates/ui-egui/src/widgets.rs` (icon buttons in `icons.rs`). Screens never
write a size, a padding or a colour as a number: a test in `theme.rs`
(`screens_take_sizes_and_colours_from_the_scale`) walks the source and fails when one does.

## The scale (`theme.rs`)

| What | Names | Values |
| --- | --- | --- |
| Headings (DM Sans bold, `theme::heading`) | `TITLE`, `SECTION`, `CARD` | 26, 18, 16 |
| Text (DM Sans regular, or medium through `theme::medium` / `label_text`) | `LEAD`, `BODY`, `SMALL`, `LABEL`, `CAPTION`, `TINY` | 17, 15, 14, 13, 12, 11 |
| Code, paths and numbers (JetBrains Mono, `theme::mono`) | `CODE` | 12.5 |
| Gaps (`ui.add_space`, item spacing) | `space::XXS` … `space::XXL` | 2, 4, 8, 12, 16, 24, 32 |
| Padding inside frames | `pad::TIGHT`, `PANEL`, `CARD`, `ROOMY`, `ROW`, `BAR`, `CHIP`, `GUTTER`, `FIELD`, `FIELD_COMPACT` | built from the gaps |
| Icons | `icon::XS` … `icon::XXL` | 12, 14, 16, 20, 24, 32 |
| Corners | `Tokens::radius_sm`, `radius`, `radius_lg` | 6 (tags, segments), 8 (controls, fields), 12 (cards, dialogs) |
| Colours | `Tokens` (`Tokens::get(ctx)`) | one set per theme; contrast is tested |

`TINY` is for the Advanced workbench's dense lists; everything else is 12 pt or more.

### Control sizes

`ControlSize` ties a control's height, label size, side padding and icon size together, so two
controls of one size always line up.

| Size | Height | Label | Icon | Used for |
| --- | --- | --- | --- | --- |
| `Small` | 24 | `LABEL` | 14 | toolbars, rows of a list, the workbench |
| `Medium` (default) | 32 | `SMALL` | 16 | buttons, switches, combo boxes, compact fields |
| `Large` | 40 | `BODY` | 16 | a screen's main switch, full text fields |
| `Hero` | 48 | `CARD` | 20 | the one action a screen is for (Start) |

egui's own controls (combo boxes, menu rows) are set up in `theme::apply` to match: `Medium` in
Simple mode, `Small` in Advanced.

## Components (`widgets.rs`)

| Need | Use |
| --- | --- |
| The main action of a screen or card | `primary`, `primary_icon` |
| Any other action | `secondary` |
| A quiet action (no box until hovered) | `ghost` |
| Something that deletes or can't be undone | `danger` |
| Another size, or a button that can be disabled | `Button::new(kind, label).icon(..).size(..).enabled(..)` |
| An icon on its own | `icons::button(ui, name, ControlSize, selected, tooltip)` |
| A link in text | `link` |
| One of a few choices | `segmented(ui, options, selected, ControlSize)`; big choices with words: `choice_tile` |
| On/off | `toggle` |
| A text field | `egui::TextEdit` with `.frame(text_field_frame(&t))` (`Large`) or `text_field_frame_compact` (`Medium`) |
| A status word | `pill`; how sure we are: `confidence_badge` |
| A box around content | `card` (screens), `panel` (workbench) |
| A section's caption | `caption` |
| A name and its value | `stat_row` |
| Nothing to show yet | `empty_state` |
| Work in progress | `Spinner`, `ProgressBar`, `working` (never egui's own: they redraw every frame) |

Every component paints its hover and pressed states, a 2 px focus ring (`focus_ring`), a
disabled state where it has one, and names itself for screen readers. Labels are translated
inside the component: pass the English key.

## Rules

- A new size, gap or colour is a change to the scale, not a number in a screen. If nothing on
  the scale fits, the design probably wants one of the existing steps.
- A size worked out from another (`body.size + 2.0`, a title bar size times its zoom) is fine.
- Padding with four different sides (a prompt box that leaves room for its buttons) is written
  as `Margin { .. }` where it is used; even padding comes from `pad`.
- Rows in a menu or a combo box's list are egui's own (`ui.button`, `selectable_label`) and take
  their look from `theme::apply`.
- The title bar follows the window's chrome (and, on macOS, keeps its size when the app zooms),
  so it scales the same tokens instead of using `ControlSize`.
- Check a change in both themes: `cargo run -p craft-anything-ui-egui --example snapshot -- --out-dir <dir>`
  renders every scene, and `--light` renders them in Studio Light.
