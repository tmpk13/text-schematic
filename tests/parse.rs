//! Parser and rule checks on small drawings. Drawings are raw strings whose
//! first newline is dropped, so the columns in the source are the columns
//! the parser sees.

use tsch::diag::Severity;
use tsch::parse_text;

fn g(s: &str) -> String {
    s.strip_prefix('\n').unwrap_or(s).to_string()
}

/// A row with each text starting at its column.
fn row(items: &[(usize, &str)]) -> String {
    let mut v: Vec<u8> = Vec::new();
    for (c, t) in items {
        if v.len() < *c {
            v.resize(*c, b' ');
        }
        for (k, b) in t.bytes().enumerate() {
            if v.len() <= c + k {
                v.push(b);
            } else {
                v[c + k] = b;
            }
        }
    }
    String::from_utf8(v).unwrap()
}

fn errors(text: &str) -> Vec<String> {
    let (_, d) = parse_text(&g(text));
    d.items.iter().filter(|x| x.sev == Severity::Error).map(|x| format!("{} {}", x.code, x.msg)).collect()
}

fn codes(text: &str) -> Vec<&'static str> {
    let (_, d) = parse_text(&g(text));
    d.items.iter().map(|x| x.code).collect()
}

fn nets(text: &str) -> Vec<String> {
    let (s, d) = parse_text(&g(text));
    assert!(!d.has_errors(), "unexpected errors: {:#?}", d.items);
    let mut v: Vec<String> = s
        .nets
        .iter()
        .map(|n| {
            let pins: Vec<String> = n.pins.iter().map(|(r, p)| format!("{r}.{p}")).collect();
            format!("{}: {}", n.names.join("="), pins.join(" "))
        })
        .collect();
    v.sort();
    v
}

#[test]
fn ldo_example_is_clean() {
    let text = std::fs::read_to_string("examples/ldo.tsch").unwrap();
    let (s, d) = parse_text(&text);
    assert!(d.items.is_empty(), "{:?}", d.items);
    assert_eq!(s.nets.len(), 3);
    let u1 = s.parts.iter().find(|p| p.reference == "U1").unwrap();
    assert_eq!(u1.value(), "AMS1117-3.3");
    let names: Vec<(&str, &str)> = u1.pins.iter().map(|p| (p.number.as_str(), p.name.as_str())).collect();
    assert!(names.contains(&("3", "VIN")) && names.contains(&("2", "VOUT")) && names.contains(&("1", "GND")));
    assert_eq!(u1.pins.iter().find(|p| p.number == "3").unwrap().etype, "power_in");
}

#[test]
fn line_through_line_crosses_without_connecting() {
    let t = r"
          +5V
           |
     R1    |
[A]--[==]--|--[B]
     1k    |
           #  R2
           #  1k
           |
          GND
";
    assert_eq!(nets(t), vec!["+5V: R2.1", "A: R1.1", "B: R1.2", "GND: R2.2"]);
}

#[test]
fn tee_needs_a_junction() {
    let tee = r"
[A]-----------[B]
       |
       #  R1
       #  1k
       |
      GND
";
    let e = errors(tee);
    assert_eq!(e.len(), 1, "{e:?}");
    assert!(e[0].starts_with("E172") && e[0].contains("without a junction"), "{e:?}");
    let fixed = tee.replacen("[A]-----------[B]", "[A]----*------[B]", 1);
    assert_eq!(nets(&fixed), vec!["A=B: R1.1", "GND: R1.2"]);
}

#[test]
fn corner_and_junction_arity() {
    let three = r"
[A]----+----[B]
       |
       #  R1
       #  1k
       |
      GND
";
    assert!(codes(three).contains(&"E173"));
    let two = r"
[A]----*
       |
       #  R1
       #  1k
       |
      GND
";
    assert!(codes(two).contains(&"E174"));
    let corner = r"
[A]----+
       |
       #  R1
       #  1k
       |
      GND
";
    assert_eq!(nets(corner), vec!["A: R1.1", "GND: R1.2"]);
}

#[test]
fn box_pins_pad_to_the_longest_number() {
    let ok = r"
       U1
       X
       +-----+
     x-1|A    |
  [P]-10|B    |
       +-----+
";
    let ok = ok.replace("     x-1|A", "     x-1|A").replace("       U1", "        U1").replace("       X\n", "        X\n");
    let ok = ok.replace("       +-----+", "        +-----+");
    let (s, d) = parse_text(&g(&ok));
    assert!(!d.has_errors(), "{:#?}", d.items);
    let u1 = &s.parts[0];
    assert_eq!(u1.pins.len(), 2);
    assert!(u1.pins.iter().all(|p| (p.length - 3.0 * 1.27).abs() < 1e-9));
    assert_eq!(s.no_connects.len(), 1);
    // Without the padding the x is not on the tip.
    let bad = ok.replace("     x-1|A", "      x1|A");
    assert!(codes(&bad).contains(&"E112"), "{:?}", codes(&bad));
}

#[test]
fn top_and_bottom_pins() {
    let t = r"
    +5V
     |
U1  8|
+---------+
|   VCC   |
|   GND   |
+---------+
    4|
     |
    GND
";
    assert_eq!(nets(t), vec!["+5V: U1.8", "GND: U1.4"]);
    let (s, _) = parse_text(&g(t));
    let names: Vec<(String, String)> = s.parts[0].pins.iter().map(|p| (p.number.clone(), p.name.clone())).collect();
    assert!(names.contains(&("8".into(), "VCC".into())) && names.contains(&("4".into(), "GND".into())), "{names:?}");
}

#[test]
fn fields_stack_above_a_box() {
    let t = r"
   U1
   LM358
   MFR-PN-1
   +-----+
  x1|A    |
   +-----+
";
    let t = t.replace("  x1|A    |", " x1|A    |");
    let (s, d) = parse_text(&g(&t));
    assert!(!d.has_errors(), "{:#?}", d.items);
    let p = &s.parts[0];
    assert_eq!(p.reference, "U1");
    assert_eq!(p.value(), "LM358");
    assert_eq!(p.field("MPN").unwrap().value, "MFR-PN-1");
}

#[test]
fn field_between_two_parts_is_ambiguous() {
    // "R12" spans the last column of one resistor and the first of the other.
    let t = r"
[A]--[==]-[==]--[B]
        R12
";
    assert!(codes(t).contains(&"E151"), "{:?}", codes(t));
}

#[test]
fn reference_letter_breaks_ties() {
    let t = r"
     R1  C1
[A]--[==]-||-[B]
";
    let (s, d) = parse_text(&g(t));
    assert!(!d.has_errors(), "{:#?}", d.items);
    let refs: Vec<&str> = s.parts.iter().map(|p| p.reference.as_str()).collect();
    assert_eq!(refs.len(), 2);
    assert!(refs.contains(&"R1") && refs.contains(&"C1"));
}

#[test]
fn diagonal_text_is_not_a_field() {
    let t = r"
         R1
[A]--[==]--[B]
";
    // "R1" starts one column right of the resistor's last cell.
    assert!(codes(t).contains(&"E152"), "{:?}", codes(t));
}

#[test]
fn power_ports_must_be_rails() {
    let t = r"
 R5
  |
  #  R1
  #  1k
  |
 GND
";
    assert!(codes(t).contains(&"E143"));
    let ok = t.replacen(" R5", "VCC", 1);
    assert_eq!(nets(&ok), vec!["GND: R1.2", "VCC: R1.1"]);
}

#[test]
fn text_may_not_touch_a_body() {
    let t = r"
  +5V
   |
   #R1
   #1k
   |
  GND
";
    assert!(codes(t).contains(&"E131"), "{:?}", codes(t));
}

#[test]
fn unconnected_pin_is_reported() {
    let t = r"
     R1
[A]--[==]
     1k
";
    let e = errors(t);
    assert!(e.iter().any(|x| x.contains("R1.2")), "{e:?}");
}

#[test]
fn glyph_pin_numbers_follow_kicad() {
    // >| is anode left. KiCad's diode has pin 1 = K, pin 2 = A.
    let t = r"
     D1
[A]-->|--[K]
";
    assert_eq!(nets(t), vec!["A: D1.2", "K: D1.1"]);
    let t = r"
     D1
[K]--|<--[A]
";
    assert_eq!(nets(t), vec!["A: D1.2", "K: D1.1"]);
}

#[test]
fn vertical_polarized_cap_plus_on_top() {
    let t = r"
 +5V
  |
  +   C1
 ===  10u
  |
 GND
";
    assert_eq!(nets(t), vec!["+5V: C1.1", "GND: C1.2"]);
    let (s, _) = parse_text(&g(t));
    assert_eq!(s.parts[0].shape_name, "CP_V_PT");
}

#[test]
fn every_glyph_parses() {
    let wire = "[A]--[==]-||-+||--||+--~~~~--->|--|<--[B]";
    let refs = row(&[(5, "R1"), (10, "C1"), (14, "C2"), (18, "C3"), (23, "L1"), (30, "D1"), (34, "D2")]);
    let t = format!("{refs}\n{wire}\n");
    let (s, d) = parse_text(&t);
    assert!(!d.has_errors(), "{:#?}", d.items);
    assert_eq!(s.parts.len(), 7);
    assert_eq!(s.nets.len(), 8);
}

#[test]
fn duplicate_reference() {
    let t = r"
     R1      R1
[A]--[==]----[==]--[B]
";
    assert!(codes(t).contains(&"E154"));
}

#[test]
fn labels_join_by_name() {
    let wire = "[A]--[==]--N1    N1--[==]--[B]";
    let t = format!("{}\n{wire}\n", row(&[(5, "R1"), (21, "R2")]));
    assert_eq!(nets(&t), vec!["A: R1.1", "B: R2.2", "N1: R1.2 R2.1"]);
}

#[test]
fn label_names_keep_their_dashes() {
    // "USB_D-" right of a wire end keeps its minus. Left of a wire the
    // minus reads as wire, so that label must be a global [USB_D-].
    let wire = "[A]--[==]--USB_D-   [USB_D-]--[==]--[B]";
    let t = format!("{}\n{wire}\n", row(&[(5, "R1"), (30, "R2")]));
    let n = nets(&t);
    assert!(n.iter().any(|x| x.starts_with("USB_D-:")), "{n:?}");
}

#[test]
fn global_label_names_have_no_spaces() {
    let t = r"
       R1
[A B]--[==]--[C]
";
    assert!(codes(t).contains(&"E146"), "{:?}", codes(t));
}

#[test]
fn stray_characters_are_errors() {
    let t = r"
     R1
[A]--[==]--[B]   ==
";
    assert!(codes(t).contains(&"E170"), "{:?}", codes(t));
}

#[test]
fn tabs_and_unicode_are_rejected() {
    assert!(codes("\tR1\n").contains(&"E002"));
    assert!(codes("R1 \u{3a9}\n").contains(&"E003"));
}

#[test]
fn header_errors_point_at_the_line() {
    let t = "+++\ntitle = \"x\"\nbogus = 1\n+++\n";
    let (_, d) = parse_text(t);
    let e = d.items.iter().find(|x| x.code == "E011").expect("header error");
    assert_eq!(e.line, 3);
}

#[test]
fn header_value_must_match_drawing() {
    let t = r#"
+++
[part.R1]
value = "2k"
+++
     R1
[A]--[==]--[B]
     1k
"#;
    assert!(codes(t).contains(&"E158"));
}

#[test]
fn value_under_another_parts_reference_is_refused() {
    // "100n" is stacked under C1's reference but sits beside C2's body.
    let t = r"
 +5V        +5V
  |          |
 ===  C1     +
  |   100n  ===  C2
 GND         |   10u
            GND
";
    let c = codes(t);
    assert!(c.contains(&"E156") || c.contains(&"E151"), "{c:?}");
}

#[test]
fn text_under_a_part_beats_a_pin_stub_row() {
    // BZX5V1 is under the diode and two columns from the capacitor's stub row.
    let wire = "[A]-*---|<---[B]";
    let t = format!(
        "{}\n{wire}\n{}\n{}\n{}\n",
        row(&[(8, "D3")]),
        row(&[(4, "|"), (7, "BZX5V1")]),
        row(&[(3, "==="), (8, "C7")]),
        row(&[(4, "|"), (8, "10u")])
    );
    let t = format!("{t}{}\n", row(&[(3, "GND")]));
    let (s, d) = parse_text(&t);
    assert!(!d.has_errors(), "{:#?}\n{t}", d.items);
    let d3 = s.parts.iter().find(|p| p.reference == "D3").unwrap();
    assert_eq!(d3.value(), "BZX5V1");
}

#[test]
fn units_share_a_reference() {
    let t = std::fs::read_to_string("examples/opamp.tsch").unwrap();
    let (s, d) = parse_text(&t);
    assert!(!d.has_errors(), "{:#?}", d.items);
    let shown: Vec<String> = s.parts.iter().map(|p| p.shown_reference()).collect();
    assert_eq!(shown, vec!["U1A", "U1B", "U1C"]);
    assert!(s.parts.iter().all(|p| p.reference == "U1"));
}

#[test]
fn unit_letters_only_on_boxes() {
    let t = r"
     R1A
[A]--[==]--[B]
";
    assert!(codes(t).contains(&"E163"), "{:?}", codes(t));
}
