//! Android compiled resource tables (`resources.arsc`): the global string pool, packages, type
//! and key string pools, type specs, and typed entries per configuration (locale, density,
//! orientation, SDK level…). The reader comes from `craft-android-res` in craft-libs.

use crate::FormatError;

pub use craft_android_res::arsc::{BagItem, Entry, Limits, Package, Table, TypeInfo};

/// Decode a resource table within `limits`.
pub fn parse(data: &[u8], limits: Limits) -> Result<Table, FormatError> {
    craft_android_res::arsc::parse(data, limits).map_err(FormatError::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use craft_anything_testkit::android as fx;

    #[test]
    fn hostile() {
        let s = fx::resources_arsc();
        for m in craft_anything_testkit::mutations(&s) {
            let _ = parse(&m, Limits { max_entries: 1000 });
        }
        for len in 0..s.len() {
            let _ = parse(&s[..len], Limits::default());
        }
        for seed in 0..64 {
            let mut n = craft_anything_testkit::noise(seed, 1024);
            n[0] = 2;
            n[1] = 0;
            n[2] = 12;
            n[3] = 0;
            n[4..8].copy_from_slice(&1024u32.to_le_bytes());
            n[12] = 0;
            n[13] = 2;
            let _ = parse(&n, Limits::default());
        }
    }
}
