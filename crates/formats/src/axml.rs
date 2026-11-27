//! Android binary XML (AXML): the compiled form of `AndroidManifest.xml` and `res/*.xml`.
//! The wire readers (chunks, the string pool, typed values, elements and attributes) come from
//! `craft-android-res` in craft-libs; this module keeps the manifest facts (package, SDK levels,
//! permissions, components) on top.

use crate::FormatError;
use serde::Serialize;

pub use craft_android_res::axml::{ANDROID_NS, Attribute, Document, Node, RES_STRING_POOL, RES_XML, ResValue, StringPool, chunk_header, framework_attr};

/// Deepest element nesting the manifest walk tracks, as deep as the reader keeps.
const MAX_DEPTH: usize = 512;

/// Decode a compiled XML document.
pub fn parse(data: &[u8]) -> Result<Document, FormatError> {
    craft_android_res::axml::parse(data).map_err(FormatError::from)
}

// ---------------------------------------------------------------------------------------------
// Manifest facts

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct IntentFilter {
    pub actions: Vec<String>,
    pub categories: Vec<String>,
    /// scheme://host/path pieces from `<data>`.
    pub data: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Component {
    /// activity, activity-alias, service, receiver, provider
    pub kind: String,
    pub name: String,
    /// The explicit `android:exported` value, when set.
    pub exported_attribute: Option<bool>,
    /// Effective: explicit value, or implied by intent filters (only before API 31).
    pub exported: bool,
    pub permission: Option<String>,
    /// A provider's separate read and write permissions.
    pub read_permission: Option<String>,
    pub write_permission: Option<String>,
    /// The explicit `android:enabled` value, when set.
    pub enabled: Option<bool>,
    pub authorities: Option<String>,
    pub intent_filters: Vec<IntentFilter>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct DeclaredPermission {
    pub name: String,
    pub protection_level: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Manifest {
    pub package: Option<String>,
    pub version_code: Option<String>,
    pub version_name: Option<String>,
    pub min_sdk: Option<String>,
    pub target_sdk: Option<String>,
    pub max_sdk: Option<String>,
    pub uses_permissions: Vec<String>,
    pub declared_permissions: Vec<DeclaredPermission>,
    pub uses_features: Vec<String>,
    pub application: ApplicationFlags,
    pub components: Vec<Component>,
    pub uses_libraries: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct ApplicationFlags {
    pub name: Option<String>,
    pub label: Option<String>,
    /// Guards every component that doesn't name its own.
    pub permission: Option<String>,
    /// The explicit `android:enabled` value, when set.
    pub enabled: Option<bool>,
    pub debuggable: Option<bool>,
    pub allow_backup: Option<bool>,
    pub uses_cleartext_traffic: Option<bool>,
    pub network_security_config: Option<String>,
    pub extract_native_libs: Option<bool>,
    pub test_only: Option<bool>,
}

fn attr<'a>(attrs: &'a [Attribute], name: &str) -> Option<&'a Attribute> {
    attrs.iter().find(|a| a.name == name && a.namespace.as_deref().is_none_or(|n| n == ANDROID_NS))
}

fn sval(attrs: &[Attribute], name: &str) -> Option<String> {
    attr(attrs, name).map(|a| a.value.clone())
}

/// Read a boolean attribute, accepting both `TYPE_INT_BOOLEAN` (`0x12`) and integer-typed
/// `Res_value` encodings (`0x10..=0x1f`, non-zero = `true`) as Android's `ResTable` does.
fn bval(attrs: &[Attribute], name: &str) -> Option<bool> {
    attr(attrs, name).and_then(|a| match a.raw.data_type {
        0x10..=0x1f => Some(a.raw.data != 0),
        _ => match a.value.as_str() {
            "true" => Some(true),
            "false" => Some(false),
            _ => None,
        },
    })
}

fn qualify(package: &Option<String>, name: &str) -> String {
    match package {
        Some(p) if name.starts_with('.') => format!("{p}{name}"),
        Some(p) if !name.contains('.') && !name.is_empty() => format!("{p}.{name}"),
        _ => name.to_string(),
    }
}

/// Facts from a decoded `AndroidManifest.xml`.
pub fn manifest(doc: &Document) -> Manifest {
    let mut m = Manifest::default();
    let mut target_level = None;
    let mut stack: Vec<String> = Vec::new();
    let mut current: Option<Component> = None;
    let mut filter: Option<IntentFilter> = None;
    for n in &doc.nodes {
        match n {
            Node::Start { name, attributes: a, .. } => {
                let parent = stack.last().map(String::as_str).unwrap_or("");
                match (parent, name.as_str()) {
                    (_, "manifest") => {
                        m.package = a.iter().find(|x| x.name == "package" && x.namespace.is_none()).map(|x| x.value.clone());
                        m.version_code = sval(a, "versionCode");
                        m.version_name = sval(a, "versionName");
                    }
                    ("manifest", "uses-sdk") => {
                        let min = attr(a, "minSdkVersion");
                        let target = attr(a, "targetSdkVersion");
                        m.min_sdk = min.map(|sdk| sdk.value.clone());
                        m.target_sdk = target.map(|sdk| sdk.value.clone());
                        m.max_sdk = sval(a, "maxSdkVersion");
                        // Use the minimum when the target is omitted. Read typed integers before formatting.
                        target_level = target.or(min).and_then(|sdk| match sdk.raw.data_type {
                            0x10 | 0x11 => u32::try_from(sdk.raw.data as i32).ok(),
                            _ => sdk.value.parse().ok(),
                        });
                    }
                    ("manifest", "uses-permission" | "uses-permission-sdk-23" | "uses-permission-sdk-m") => {
                        if let Some(p) = sval(a, "name")
                            && m.uses_permissions.len() < 4096
                        {
                            m.uses_permissions.push(p);
                        }
                    }
                    ("manifest", "permission") => {
                        if let Some(p) = sval(a, "name")
                            && m.declared_permissions.len() < 4096
                        {
                            m.declared_permissions.push(DeclaredPermission { name: p, protection_level: sval(a, "protectionLevel") });
                        }
                    }
                    ("manifest", "uses-feature") => {
                        if let Some(p) = sval(a, "name")
                            && m.uses_features.len() < 4096
                        {
                            m.uses_features.push(p);
                        }
                    }
                    ("manifest", "application") => {
                        m.application = ApplicationFlags {
                            name: sval(a, "name").map(|n| qualify(&m.package, &n)),
                            label: sval(a, "label"),
                            permission: sval(a, "permission"),
                            enabled: bval(a, "enabled"),
                            debuggable: bval(a, "debuggable"),
                            allow_backup: bval(a, "allowBackup"),
                            uses_cleartext_traffic: bval(a, "usesCleartextTraffic"),
                            network_security_config: sval(a, "networkSecurityConfig"),
                            extract_native_libs: bval(a, "extractNativeLibs"),
                            test_only: bval(a, "testOnly"),
                        };
                    }
                    ("application", "uses-library") => {
                        if let Some(p) = sval(a, "name")
                            && m.uses_libraries.len() < 1024
                        {
                            m.uses_libraries.push(p);
                        }
                    }
                    ("application", k @ ("activity" | "activity-alias" | "service" | "receiver" | "provider")) => {
                        current = Some(Component {
                            kind: k.to_string(),
                            name: qualify(&m.package, &sval(a, "name").unwrap_or_default()),
                            exported_attribute: bval(a, "exported"),
                            exported: false,
                            permission: sval(a, "permission"),
                            read_permission: sval(a, "readPermission"),
                            write_permission: sval(a, "writePermission"),
                            enabled: bval(a, "enabled"),
                            authorities: sval(a, "authorities"),
                            intent_filters: Vec::new(),
                        });
                    }
                    (_, "intent-filter") if current.is_some() => filter = Some(IntentFilter::default()),
                    ("intent-filter", "action") => {
                        if let (Some(f), Some(v)) = (filter.as_mut(), sval(a, "name")) {
                            f.actions.push(v);
                        }
                    }
                    ("intent-filter", "category") => {
                        if let (Some(f), Some(v)) = (filter.as_mut(), sval(a, "name")) {
                            f.categories.push(v);
                        }
                    }
                    ("intent-filter", "data") => {
                        if let Some(f) = filter.as_mut() {
                            let parts: Vec<String> = ["scheme", "host", "port", "path", "pathPrefix", "pathPattern", "mimeType"]
                                .iter()
                                .filter_map(|k| sval(a, k).map(|v| format!("{k}={v}")))
                                .collect();
                            if !parts.is_empty() {
                                f.data.push(parts.join(" "));
                            }
                        }
                    }
                    _ => {}
                }
                if stack.len() < MAX_DEPTH {
                    stack.push(name.clone());
                }
            }
            Node::End { name, .. } => {
                stack.pop();
                match name.as_str() {
                    "intent-filter" => {
                        if let (Some(c), Some(f)) = (current.as_mut(), filter.take())
                            && c.intent_filters.len() < 256
                        {
                            c.intent_filters.push(f);
                        }
                    }
                    "activity" | "activity-alias" | "service" | "receiver" | "provider" => {
                        if let Some(c) = current.take()
                            && m.components.len() < 20_000
                        {
                            m.components.push(c);
                        }
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }
    for c in &mut m.components {
        // Providers were exported by default up to API 16, intent filters or not.
        let implied =
            if c.kind == "provider" { target_level.is_none_or(|t| t <= 16) } else { !c.intent_filters.is_empty() && target_level.is_none_or(|t| t < 31) };
        c.exported = c.exported_attribute.unwrap_or(implied);
    }
    m
}

#[cfg(test)]
mod tests {
    use super::*;
    use craft_anything_testkit::android as fx;

    const XML_START_ELEMENT: u16 = 0x0102;

    fn chunk_at(data: &[u8], kind: u16) -> usize {
        let mut pos = 8;
        loop {
            let (t, _, n) = chunk_header(data, pos).unwrap();
            if t == kind {
                return pos;
            }
            pos += n;
        }
    }

    #[test]
    fn nodes_cannot_borrow_from_a_sibling_or_from_after_the_document() {
        let original = fx::manifest_axml();
        let pos = chunk_at(&original, XML_START_ELEMENT);
        for (header, size) in [(8u16, 36u32), (16, 35)] {
            let mut data = original.clone();
            data[pos + 2..pos + 4].copy_from_slice(&header.to_le_bytes());
            data[pos + 4..pos + 8].copy_from_slice(&size.to_le_bytes());
            let doc = parse(&data).unwrap();
            assert!(doc.truncated);
            assert!(manifest(&doc).package.is_none());
        }
        let mut data = original;
        data[4..8].copy_from_slice(&((pos + 8) as u32).to_le_bytes());
        let doc = parse(&data).unwrap();
        assert!(doc.truncated);
        assert!(manifest(&doc).package.is_none());
    }

    #[test]
    fn attributes_need_their_own_complete_records_and_typed_values() {
        let original = fx::manifest_axml();
        let ext = chunk_at(&original, XML_START_ELEMENT) + 16;
        for (offset, val) in [(8, 0u16), (10, 4), (10, u16::MAX), (32, 7), (32, 9)] {
            let mut data = original.clone();
            data[ext + offset..ext + offset + 2].copy_from_slice(&val.to_le_bytes());
            let doc = parse(&data).unwrap();
            assert!(doc.truncated, "field {offset}, value {val}");
            assert!(manifest(&doc).package.is_none());
        }
    }

    #[test]
    fn decodes_manifest() {
        let doc = parse(&fx::manifest_axml()).unwrap();
        assert!(doc.warnings.is_empty(), "{:?}", doc.warnings);
        let m = manifest(&doc);
        assert_eq!(m.package.as_deref(), Some(fx::PACKAGE));
        assert_eq!(m.version_code.as_deref(), Some("42"));
        assert_eq!(m.version_name.as_deref(), Some("1.2.3"));
        assert_eq!(m.min_sdk.as_deref(), Some("24"));
        assert_eq!(m.target_sdk.as_deref(), Some("34"));
        assert_eq!(m.uses_permissions, vec!["android.permission.INTERNET", "android.permission.CAMERA"]);
        assert_eq!(m.declared_permissions[0].name, "com.example.notes.permission.SYNC");
        assert_eq!(m.declared_permissions[0].protection_level.as_deref(), Some("0x2"));
        assert_eq!(m.application.debuggable, Some(true));
        assert_eq!(m.application.allow_backup, Some(false));
        assert_eq!(m.application.label.as_deref(), Some("@0x7f010000"));
        assert_eq!(m.components.len(), 3);
        let main = &m.components[0];
        assert_eq!(main.name, "com.example.notes.MainActivity");
        assert!(main.exported);
        assert_eq!(main.intent_filters[0].actions, vec!["android.intent.action.MAIN"]);
        assert_eq!(main.intent_filters[0].categories, vec!["android.intent.category.LAUNCHER"]);
        assert_eq!(main.intent_filters[0].data, vec!["scheme=notes host=open"]);
        assert_eq!(m.components[1].kind, "service");
        assert!(!m.components[1].exported);
        assert_eq!(m.components[2].kind, "provider");
        assert_eq!(m.components[2].authorities.as_deref(), Some("com.example.notes.files"));
        let xml = doc.to_xml(1 << 20);
        assert!(xml.contains("<manifest xmlns:android=\"http://schemas.android.com/apk/res/android\""), "{xml}");
        assert!(xml.contains("android:name=\"android.permission.INTERNET\""), "{xml}");
    }

    #[test]
    fn hostile() {
        let s = fx::manifest_axml();
        for m in craft_anything_testkit::mutations(&s) {
            if let Ok(d) = parse(&m) {
                let _ = manifest(&d);
                let _ = d.to_xml(4096);
            }
        }
        for len in 0..s.len() {
            let _ = parse(&s[..len]);
        }
        for seed in 0..64 {
            let mut n = craft_anything_testkit::noise(seed, 512);
            n[0] = 3;
            n[1] = 0;
            n[2] = 8;
            n[3] = 0;
            n[4..8].copy_from_slice(&512u32.to_le_bytes());
            let _ = parse(&n);
        }
    }
}
