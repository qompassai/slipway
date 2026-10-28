//! Artifact census: what an APK/AAB actually contains, read from
//! the artifact itself — entry inventory, native libraries with
//! their ELF LOAD alignment, hashes. Never from config, never
//! from a previous run (publish-skill operating rule 1).

use std::io::Read;
use std::path::Path;

use serde_json::{Value, json};

use crate::exec::sha256_bytes;
use crate::model::Failure;

/// One native library inside an artifact.
pub struct NativeLib {
    pub entry: String,
    pub abi: String,
    pub bits: u32,
    /// Maximum PT_LOAD alignment (bytes), parsed from the ELF
    /// program headers. `None` when the entry is not a parseable
    /// ELF (reported as a fact, not hidden).
    pub load_align: Option<u64>,
    pub size: u64,
}

/// The census of one artifact file.
pub struct Census {
    pub sha256: String,
    pub size: u64,
    pub entries: Vec<String>,
    pub native_libs: Vec<NativeLib>,
    pub has_signature_block: bool,
}

fn elf_load_align(bytes: &[u8]) -> Option<(u32, u64)> {
    if bytes.len() < 64 || &bytes[0..4] != b"\x7fELF" {
        return None;
    }
    let is_64 = match bytes[4] {
        2 => true,
        1 => false,
        _ => return None,
    };
    let little = match bytes[5] {
        1 => true,
        2 => false,
        _ => return None,
    };
    let u16_at = |off: usize| -> Option<u16> {
        let b = bytes.get(off..off + 2)?;
        Some(if little {
            u16::from_le_bytes([b[0], b[1]])
        } else {
            u16::from_be_bytes([b[0], b[1]])
        })
    };
    let u32_at = |off: usize| -> Option<u32> {
        let b = bytes.get(off..off + 4)?;
        Some(if little {
            u32::from_le_bytes([b[0], b[1], b[2], b[3]])
        } else {
            u32::from_be_bytes([b[0], b[1], b[2], b[3]])
        })
    };
    let u64_at = |off: usize| -> Option<u64> {
        let b = bytes.get(off..off + 8)?;
        Some(if little {
            u64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]])
        } else {
            u64::from_be_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]])
        })
    };
    let mut max_align: u64 = 0;
    if is_64 {
        let phoff = u64_at(32)? as usize;
        let phentsize = u16_at(54)? as usize;
        let phnum = u16_at(56)? as usize;
        if phentsize < 56 {
            return None;
        }
        for i in 0..phnum.min(128) {
            let off = phoff.checked_add(i.checked_mul(phentsize)?)?;
            if u32_at(off)? != 1 {
                continue; // not PT_LOAD
            }
            let align = u64_at(off + 48)?;
            max_align = max_align.max(align);
        }
        Some((64, max_align))
    } else {
        let phoff = u32_at(28)? as usize;
        let phentsize = u16_at(42)? as usize;
        let phnum = u16_at(44)? as usize;
        if phentsize < 32 {
            return None;
        }
        for i in 0..phnum.min(128) {
            let off = phoff.checked_add(i.checked_mul(phentsize)?)?;
            if u32_at(off)? != 1 {
                continue;
            }
            let align = u64::from(u32_at(off + 28)?);
            max_align = max_align.max(align);
        }
        Some((32, max_align))
    }
}

/// Build the census for one artifact (APK or AAB — both are zips).
pub fn census(path: &Path) -> Result<Census, Failure> {
    let bytes = std::fs::read(path)
        .map_err(|e| Failure::domain("io", format!("cannot read {}: {e}", path.display())))?;
    let sha256 = sha256_bytes(&bytes);
    let size = bytes.len() as u64;
    let cursor = std::io::Cursor::new(bytes);
    let mut archive = zip::ZipArchive::new(cursor).map_err(|e| {
        Failure::domain(
            "not_an_artifact",
            format!("{} is not a zip-form artifact: {e}", path.display()),
        )
    })?;
    let mut entries = Vec::new();
    let mut native_libs = Vec::new();
    let mut has_signature_block = false;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).map_err(|e| {
            Failure::domain("not_an_artifact", format!("zip entry {i} unreadable: {e}"))
        })?;
        let name = entry.name().to_string();
        entries.push(name.clone());
        let upper = name.to_uppercase();
        if upper.starts_with("META-INF/")
            && (upper.ends_with(".RSA") || upper.ends_with(".DSA") || upper.ends_with(".EC"))
        {
            has_signature_block = true;
        }
        if let Some(rest) = name.strip_prefix("lib/")
            && name.ends_with(".so")
        {
            let mut content = Vec::new();
            let entry_size = entry.size();
            if entry.read_to_end(&mut content).is_ok() {
                let parsed = elf_load_align(&content);
                native_libs.push(NativeLib {
                    abi: rest.split('/').next().unwrap_or("").to_string(),
                    entry: name.clone(),
                    bits: parsed.map(|(b, _)| b).unwrap_or(0),
                    load_align: parsed.map(|(_, a)| a),
                    size: entry_size,
                });
            }
        }
        // AAB layout: native libs live under base/lib/.
        if let Some(rest) = name.strip_prefix("base/lib/")
            && name.ends_with(".so")
        {
            let mut content = Vec::new();
            let entry_size = entry.size();
            if entry.read_to_end(&mut content).is_ok() {
                let parsed = elf_load_align(&content);
                native_libs.push(NativeLib {
                    abi: rest.split('/').next().unwrap_or("").to_string(),
                    entry: name.clone(),
                    bits: parsed.map(|(b, _)| b).unwrap_or(0),
                    load_align: parsed.map(|(_, a)| a),
                    size: entry_size,
                });
            }
        }
    }
    entries.sort();
    Ok(Census {
        sha256,
        size,
        entries,
        native_libs,
        has_signature_block,
    })
}

/// The per-.so 16 KB alignment table for the 64-bit libraries
/// (the Play gate, from 2027-02-01): each entry's measured LOAD
/// alignment and whether it meets `0x4000`.
pub fn alignment_table(census: &Census) -> Value {
    let rows: Vec<Value> = census
        .native_libs
        .iter()
        .map(|lib| {
            json!({
                "entry": lib.entry,
                "abi": lib.abi,
                "bits": lib.bits,
                "size": lib.size,
                "load_align": lib.load_align.map(|a| format!("0x{a:x}")),
                "meets_16k": lib.bits == 64 && lib.load_align.is_some_and(|a| a >= 0x4000),
                "in_16k_scope": lib.bits == 64,
            })
        })
        .collect();
    let scoped: Vec<&NativeLib> = census.native_libs.iter().filter(|l| l.bits == 64).collect();
    json!({
        "libs": rows,
        "libs_64bit": scoped.len(),
        "all_64bit_meet_16k": !scoped.is_empty()
            && scoped.iter().all(|l| l.load_align.is_some_and(|a| a >= 0x4000)),
    })
}

/// PNG facts from the IHDR header, when the file is a PNG.
pub struct PngFacts {
    pub width: u32,
    pub height: u32,
    pub bit_depth: u8,
    /// IHDR color type: 2 = truecolor (no alpha), 3 = palette,
    /// 4 = gray+alpha, 6 = RGBA (alpha).
    pub color_type: u8,
}

impl PngFacts {
    pub fn has_alpha(&self) -> bool {
        matches!(self.color_type, 4 | 6)
    }
    /// The Play listing-image rule: 24-bit PNG, no alpha.
    pub fn is_24bit_no_alpha(&self) -> bool {
        self.color_type == 2 && self.bit_depth == 8
    }
}

pub fn png_facts(bytes: &[u8]) -> Option<PngFacts> {
    if bytes.len() < 33 || &bytes[0..8] != b"\x89PNG\r\n\x1a\n" || &bytes[12..16] != b"IHDR" {
        return None;
    }
    let width = u32::from_be_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]);
    let height = u32::from_be_bytes([bytes[20], bytes[21], bytes[22], bytes[23]]);
    Some(PngFacts {
        width,
        height,
        bit_depth: bytes[24],
        color_type: bytes[25],
    })
}

pub fn is_jpeg(bytes: &[u8]) -> bool {
    bytes.len() > 3 && bytes[0] == 0xFF && bytes[1] == 0xD8 && bytes[2] == 0xFF
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A synthetic ELF64 with one PT_LOAD at the given alignment.
    fn fake_elf(align: u64) -> Vec<u8> {
        let mut elf = vec![0u8; 64 + 56];
        elf[0..4].copy_from_slice(b"\x7fELF");
        elf[4] = 2; // 64-bit
        elf[5] = 1; // little-endian
        elf[32..40].copy_from_slice(&64u64.to_le_bytes()); // phoff
        elf[54..56].copy_from_slice(&56u16.to_le_bytes()); // phentsize
        elf[56..58].copy_from_slice(&1u16.to_le_bytes()); // phnum
        elf[64..68].copy_from_slice(&1u32.to_le_bytes()); // PT_LOAD
        elf[64 + 48..64 + 56].copy_from_slice(&align.to_le_bytes());
        elf
    }

    #[test]
    fn elf_alignment_parses() {
        assert_eq!(elf_load_align(&fake_elf(0x4000)), Some((64, 0x4000)));
        assert_eq!(elf_load_align(&fake_elf(0x1000)), Some((64, 0x1000)));
        assert_eq!(elf_load_align(b"not an elf"), None);
        assert_eq!(elf_load_align(&fake_elf(0x4000)[..40]), None);
    }

    fn png_header(color_type: u8, bit_depth: u8) -> Vec<u8> {
        let mut png = Vec::new();
        png.extend_from_slice(b"\x89PNG\r\n\x1a\n");
        png.extend_from_slice(&13u32.to_be_bytes());
        png.extend_from_slice(b"IHDR");
        png.extend_from_slice(&1080u32.to_be_bytes());
        png.extend_from_slice(&2400u32.to_be_bytes());
        png.push(bit_depth);
        png.push(color_type);
        png.extend_from_slice(&[0, 0, 0]);
        png.extend_from_slice(&[0, 0, 0, 0]);
        png
    }

    #[test]
    fn png_alpha_detection() {
        let rgba = png_facts(&png_header(6, 8)).expect("png");
        assert!(rgba.has_alpha());
        assert!(!rgba.is_24bit_no_alpha());
        let rgb = png_facts(&png_header(2, 8)).expect("png");
        assert!(!rgb.has_alpha());
        assert!(rgb.is_24bit_no_alpha());
        assert!(png_facts(b"\x89PNG").is_none());
    }
}
