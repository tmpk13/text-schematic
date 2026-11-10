# Trials

Each trial gives a fresh Claude agent a circuit and only the tool: the
format comes from `tsch spec` and `--help`, never from the source, the
examples or the tests. The question is whether drawing in text helps an LLM
produce a correct, readable schematic.

## 2026-11-10: ESP32-C3 USB-C board

- Agent: a fresh general-purpose Claude subagent (Opus 5.5), 26 tool calls,
  about 12 minutes.
- Task: USB-C receptacle with CC pull-downs, AP2112K LDO with input and
  output caps, ESP32-C3-MINI-1 module with EN RC, BOOT button with pull-up,
  status LED, I2C header with pull-ups, USB data to the module; footprints
  and MPN lines. 16 parts, 13 nets. The result is `examples/esp32c3.tsch`.
- Result: `check` went 79 errors, then 2, then 0. The first `netlist` matched
  the intended circuit on all 13 nets. `verify` passed on its first run:
  KiCad's netlist equal to the drawn one, ERC 0 errors and 0 warnings, 115
  texts inside their cells.
- Every error was column alignment from typing long lines by hand. Twelve
  of 40 lines had something off by one or two columns. One broken box side
  produced 78 follow-on errors that hid the real one (E102).
- One rule surprise: the value `Conn_01x04` written above a box had its `x`
  read as an unused top pin.
- The agent's own assessment: the layout it planned is exactly what was
  rendered, and verify proving no overlaps was valuable. In practice it
  still planned on a coordinate grid and transcribed it; typing exact runs of
  spaces was the hard part, followed by keeping field lines clear of
  neighboring parts. It judged the effort about the same as placing by
  coordinates, with a result far easier to review.
- Changes made in response: a broken box is reported first and parsing
  continues as if it were closed (79 findings become about 10, all on the
  broken row); `x` counts as a no-connect only standing alone on a pin tip;
  pin numbers are letters and digits only; `tsch tokens`, `tsch shift` and
  `tsch put` for column-exact editing; the spec now covers x in words,
  letter pin numbers, header-only values, power port alignment, field
  matching and ERC power rules.
- Caveat: the tool binary was rebuilt once during the run, adding
  multi-unit parts; nothing the agent used changed behavior.

Not yet measured: the same circuit drawn by agents using coordinate tools
or a layout engine, scored the same way. That comparison is what would
answer the question rather than suggest an answer.
