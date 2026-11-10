# Text schematic format (.tsch)

A `.tsch` file draws a schematic as a grid of characters. Each character is
one cell of the KiCad sheet, 1.27 mm wide and 2.54 mm tall. `tsch build`
turns it into a KiCad 10 schematic where every line, part and text sits in
the cells where it was typed. Text is drawn in a font whose characters are
exactly one cell wide, so a value typed in columns 20-23 is drawn in columns
20-23. Nothing may overlap: every cell belongs to one element.

## File layout

```
+++
title = "3.3 V LDO"            # optional TOML header between +++ lines
[part.U1]
footprint = "Package_TO_SOT_SMD:SOT-223-3_TabPin2"
+++
<drawing: every line after the header is a grid row>
```

The drawing is ASCII only, with no tabs. Positions in messages are file
`line:column`, 1-based. `tsch show FILE --lines 20:60` prints part of a file
with a column ruler.

## Cells, text and drawing

- Wire characters: `-` `|` `+` `*`. Part characters: see Boxes and Glyphs.
- Text is any run of characters that contains a letter or digit. Words
  separated by one space are one text ("100n 50V"); two or more spaces
  separate texts. `|` never belongs to text.
- Text may touch drawing only where noted below (pin names against a box
  edge, pin numbers, labels at wire ends). Elsewhere leave a space.
- One text row is one cell tall, so text lines can sit in adjacent rows.

## Wires

```
 ------      horizontal wire          |      vertical wire
                                      |
 ---+        corner: + joins exactly 2 wires (or a wire and a pin)
    |
 ---*---     junction: * joins 3 or 4 wires; KiCad draws a dot
    |
    |
 ---|---     crossing: a line drawn straight through another line,
    |        both ways, crosses it without connecting
```

- Every wire end must land on something: a pin tip, a label, a power port,
  a corner or a junction. A dangling end is an error.
- A vertical wire that ends on a horizontal wire (or the reverse) is an
  error: write `*` to join, or continue the line through to cross.

## Boxes (ICs, connectors, modules)

```
       U1                         reference and value: see Fields
       AMS1117-3.3
       +-----------+
  ----3|VIN    VOUT|2----         left and right pins
       |    GND    |
       +-----------+
            1|                    bottom pin: number left of the pin line
             |
            GND
```

- Outline: `+` corners, `-` top and bottom, `|` sides, at least 3 x 3. Inside
  holds only text.
- Left and right pins: the pin number is written against the edge, outside
  it (`3|`, `|2`). All left and right pins of a box have one length: its
  longest left/right pin number plus one cell. The tip is the cell that far
  from the edge and holds a wire character (`-` `+` `*`) or `x`; cells
  between the number and the tip are `-`. In a box whose longest number has
  two digits: `-12|`, `--3|`, `x-3|` (unused), `|12-`, `|3--`.
- Top and bottom pins: a `|` (or `x`) directly above the top edge or below
  the bottom edge, with the number written immediately left of it (`8|`).
  That `|` is the pin tip; the wire continues from it.
- Pin names: a left pin's name starts in the cell right after the edge
  (`|VIN`); a right pin's name ends right before it (`VOUT|`). A top or
  bottom pin's name is in the row just inside the edge, covering the pin's
  column. Names are optional; other text inside the box (indented by a
  space) is drawn as is.
- Pin numbers in one box are unique. Pin electrical types default to
  passive; set them in the header (`pins = { 1 = "power_in", VOUT = "power_out" }`).

## Two-pin glyphs

Each glyph sits between two pin tips: the cells just outside it along its
axis, which must hold a wire character or `x`.

- Resistor: `[=]` .. `[====]` horizontal; `#` stacked 1-4 rows vertical.
  Pin 1 left / top.
- Capacitor: `||` horizontal; `===` vertical, the wire in its middle
  column. Pin 1 left / top.
- Polarized capacitor: `+||` or `||+` horizontal; a `+` directly above or
  below `===` vertical. Pin 1 is the `+` side.
- Inductor: `~~` .. `~~~~~~~~` horizontal; `)` or `(` stacked 1-4 rows
  vertical. Pin 1 left / top.
- Diode: `>|` (anode left) or `|<` horizontal; `V` (anode top) or `^`
  vertical. Pins as KiCad numbers them: 1 = cathode, 2 = anode.

```
  --[==]--   --||--   --+||--   --~~~~--   -->|--      |     |     |     |
                                                      #    ===    +     V
                                                      #     |    ===    |
                                                      |          |
```

- A vertical resistor, inductor, diode or `+` marker needs the cell on each
  side blank (its graphics are wider than one column).
- Variants (header `symbol = ...`): diodes `led`, `zener`, `schottky`;
  inductors `ferrite`; resistors `fuse`. They change the drawing in the
  same cells.
- Anything else (transistors, crystals, switches, connectors) is a box.

## Fields (reference, value, part number)

Fields are the text lines next to a part. A line belongs to a part when it

- shares a row with the part, at most 2 blank columns away, or
- sits in the row directly above or below the part and shares at least
  one column with it, or
- stacks left-aligned directly above or below a line that belongs to it.

Diagonal positions do not count. Beside a vertical glyph its pin tip rows
also count, but only when the line touches no other part more directly. A
value line aligned directly under another part's field line is an error.

Lines are read top to bottom, then left to right. The first is the
reference (`R1`, `U3`, `LED2`: capital letters then digits), the second the
value, the rest are named by the part's `visible = [...]` list (default
`["MPN"]`). Every part needs a drawn reference. A line next to two parts is
an error; a reference goes to a glyph whose letter matches (R, C, L, D...)
before a box. Usual places:

```
  R1                          U1                 |
--[==]--      ref above,      LM358              #  R2        fields right of a
  10k         value below     +-----+            #  4k7       vertical part
                                                 |  RC0603FR-074K7L
```

## Nets

- Local label: text touching a wire end, `---SDA` or `SDA---`. Connects to
  labels with the same name on the same sheet. A name ending in `-`
  (`USB_D-`) works only right of the wire; use a global label otherwise.
- Global label: `---[SDA]` or `[SDA]---`. Connects across sheets. Drawn as
  a flag around the name.
- Power port: a name directly above a vertical wire end (supply,
  `+3V3` over `|`) or directly below one (ground, `GND` under `|`). Ground
  names (GND, AGND, VSS...) get the ground symbol. Port names must look like
  rails (GND*, VSS*, +3V3, 5V, 1V8, VCC*, VDD*, VBUS, VIN, ...) or be listed
  in the header `power = ["VMAIN"]`. Ports connect across sheets.
- No-connect: `x` on a pin tip marks the pin unused.
- Vertical wire ends take power ports only; give a signal a horizontal
  stub and a label.

## Header reference

```toml
title = "Sheet title"         # title block; also date, rev, company
comments = ["line 1"]         # title block comments
paper = "A3"                  # A4..A0; default: smallest that fits
power = ["VMAIN"]             # extra names allowed as power ports

[default.R]                   # every part whose reference starts with R
footprint = "Resistor_SMD:R_0603_1608Metric"

[part.U1]
value = "AMS1117-3.3"         # if also drawn, both must match
footprint = "Package_TO_SOT_SMD:SOT-223-3_TabPin2"
datasheet = "https://..."
description = "LDO 3.3 V 1 A"
fields = { MPN = "AMS1117-3.3", Manufacturer = "AMS" }   # hidden unless drawn
visible = ["MPN"]             # names of drawn lines after the value
pins = { 1 = "power_in", VOUT = "power_out" }            # by number or name
symbol = "led"                # glyph variant
dnp = true                    # also in_bom, on_board
```

Pin types: input, output, bidirectional, tri_state, passive, free,
unspecified, power_in, power_out, open_collector, open_emitter, no_connect.

## Several sheets

A project file lists the sheets; `tsch build project.toml` writes a root
page with one block per sheet. Sheets connect through global labels and
power ports; references are unique across the project.

```toml
name = "usb-board"
title = "USB to 3.3 V MCU board"
sheets = ["power.tsch", "mcu.tsch"]
```

## Commands

```
tsch check FILE|project.toml         rules; errors and warnings with hints
tsch build FILE -o out               write .kicad_sch, .kicad_pro, tsch.kicad_sym
tsch verify FILE -o out              build, then prove it with kicad-cli
tsch netlist FILE                    nets as the drawing defines them
tsch show FILE --lines A:B --cols A:B   print with a column ruler
tsch render FILE -o out              PNG of each sheet
```

`verify` passes when KiCad's exported netlist equals the drawn one, ERC has
no errors, and in the plotted sheet every text lies inside the cells it was
typed in with no text overlapping another and no line crossing a text.

## Working method

1. Plan the page: signal flow left to right, supplies at the top, grounds
   at the bottom. Leave blank columns and rows between blocks; a sheet is
   cheap to make wider (A4 holds about 210 columns by 72 rows).
2. Draw boxes first, then glyphs, wires, labels and power ports, then
   fields.
3. Run `tsch check` and fix what it reports; each message names the cell.
4. Read `tsch netlist` against the intended circuit.
5. Run `tsch verify`; look at `tsch render` output for layout.
