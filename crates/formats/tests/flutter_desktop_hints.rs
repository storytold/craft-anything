//! Desktop Flutter builds are named by their engine import, and only weakly by text (#2227).

use craft_anything_formats::{hints::detect, parse_binary};
use craft_anything_testkit as tk;

fn flutter(data: &[u8]) -> Option<craft_anything_formats::hints::Hint> {
    let b = parse_binary(data).unwrap();
    detect(&b, data).into_iter().find(|h| h.id == "flutter")
}

#[test]
fn a_windows_runner_importing_flutter_windows_dll_is_flutter() {
    let pe = tk::flutter::flutter_windows_exe();
    let b = parse_binary(&pe).unwrap();
    assert!(b.libraries.iter().any(|l| l.eq_ignore_ascii_case("flutter_windows.dll")), "{:?}", b.libraries);
    let h = flutter(&pe).expect("a Flutter hint");
    assert_eq!(h.strength, "strong", "{h:?}");
    assert!(h.evidence.contains("flutter_windows.dll"), "{h:?}");
}

#[test]
fn a_linux_runner_needing_the_flutter_engine_is_flutter() {
    let elf = tk::flutter::flutter_linux_exe();
    let b = parse_binary(&elf).unwrap();
    assert!(b.libraries.iter().any(|l| l == "libflutter_linux_gtk.so"), "{:?}", b.libraries);
    let h = flutter(&elf).expect("a Flutter hint");
    assert_eq!(h.strength, "strong", "{h:?}");
    assert!(h.evidence.contains("libflutter_linux_gtk.so"), "{h:?}");
}

#[test]
fn a_macos_runner_loading_the_flutter_engine_is_flutter() {
    let macho = tk::flutter::flutter_macos_exe();
    let b = parse_binary(&macho).unwrap();
    assert!(b.libraries.iter().any(|l| l.contains("FlutterMacOS.framework")), "{:?}", b.libraries);
    let h = flutter(&macho).expect("a Flutter hint");
    assert_eq!(h.strength, "strong", "{h:?}");
    assert!(h.evidence.contains("FlutterMacOS.framework"), "{h:?}");
}

#[test]
fn flutter_text_alone_is_a_weak_hint() {
    let h = flutter(&tk::flutter::pe_mentioning_flutter()).expect("a Flutter hint");
    assert_eq!(h.strength, "weak", "{h:?}");
    assert!(h.evidence.contains("flutter_assets"), "{h:?}");
}

#[test]
fn a_plain_program_is_not_flutter() {
    assert_eq!(flutter(&tk::pe64()), None);
    let (elf, _) = tk::elf64(tk::ElfArch::X86_64, true);
    assert_eq!(flutter(&elf), None);
    assert_eq!(flutter(&tk::macho64()), None);
}
