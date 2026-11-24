use std::path::{Path, PathBuf};

use rspigot_anvil::{AnvilError, Compression};
use rspigot_nbt::{Compound, NbtError, Tag, read_named, write_named};
use thiserror::Error;

const SECTOR_BYTES: usize = 4096;
const HEADER_BYTES: usize = 8192;
const ENTRIES: usize = 1024;
const CHUNK_HEADER_BYTES: usize = 5;

#[derive(Debug, Error)]
pub enum AnvilCompatError {
    #[error("erreur E/S sur {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("erreur Anvil dans {path}: {source}")]
    Anvil {
        path: PathBuf,
        #[source]
        source: AnvilError,
    },
    #[error("erreur NBT dans {path} (slot {slot}): {source}")]
    Nbt {
        path: PathBuf,
        slot: usize,
        #[source]
        source: NbtError,
    },
    #[error("compression Anvil non supportée ({id}) dans {path}")]
    UnsupportedCompression { path: PathBuf, id: u8 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChunkNormalization {
    Unchanged,
    ConvertedPalette,
    DroppedEmptyPre118,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NormalizationSummary {
    pub files_scanned: usize,
    pub files_rewritten: usize,
    pub files_removed_empty: usize,
    pub chunks_converted: usize,
    pub chunks_unchanged: usize,
    pub chunks_dropped_empty_legacy: usize,
}

struct PackedSlot {
    compression_id: u8,
    compressed_data: Vec<u8>,
    timestamp: u32,
}

/// Normalizes a single Minecraft 26.x (`DataVersion = 5023`) block-state palette tag
/// (`Tag::String`, `{"": "..."}`, or `{"id": "...", "properties": {...}}`) into the
/// `{"Name": "...", "Properties": {...}}` layout expected by `rspigot-anvil`, and marks
/// all `*_leaves` blocks as `persistent = "true"` so custom lobby trees never decay.
pub fn normalize_palette_entry(tag: &mut Tag) -> bool {
    let mut changed = match tag {
        Tag::String(name) => {
            let block_name = std::mem::take(name);
            *tag = Tag::Compound(Compound::new().with("Name", block_name));
            true
        }
        Tag::Compound(comp) => {
            if comp.contains_key("Name") {
                false
            } else if let Some(Tag::String(name)) = comp.remove("") {
                *comp = Compound::new().with("Name", name);
                true
            } else if let Some(Tag::String(name)) = comp.remove("id") {
                let props = comp.remove("properties");
                let mut next = Compound::new().with("Name", name);
                if let Some(Tag::Compound(p)) = props
                    && !p.is_empty()
                {
                    next.insert("Properties", Tag::Compound(p));
                }
                *comp = next;
                true
            } else {
                false
            }
        }
        _ => return false,
    };

    if let Tag::Compound(comp) = tag
        && ensure_persistent_leaves(comp)
    {
        changed = true;
    }

    changed
}

fn ensure_persistent_leaves(comp: &mut Compound) -> bool {
    let is_leaves = comp
        .get_str("Name")
        .is_some_and(|name| name.ends_with("_leaves"));
    if !is_leaves {
        return false;
    }

    match comp.get_mut("Properties") {
        Some(Tag::Compound(props)) => {
            if props.get_str("persistent") == Some("true") {
                false
            } else {
                props.insert("persistent", Tag::String("true".to_owned()));
                true
            }
        }
        _ => {
            comp.insert(
                "Properties",
                Tag::Compound(Compound::new().with("persistent", "true")),
            );
            true
        }
    }
}

fn is_empty_pre_118_chunk(root: &Compound) -> bool {
    let Some(level) = root.get_compound("Level") else {
        return false;
    };
    let Some(sections) = level.get_list("Sections") else {
        return true;
    };
    for sec in sections {
        let Some(sec_comp) = sec.as_compound() else {
            continue;
        };
        if let Some(palette) = sec_comp.get_list("Palette") {
            for entry in palette {
                let name = entry
                    .as_compound()
                    .and_then(|c| c.get_str("Name"))
                    .unwrap_or("minecraft:air");
                if name != "minecraft:air" && name != "minecraft:cave_air" {
                    return false;
                }
            }
        }
    }
    true
}

pub fn normalize_chunk_nbt(root: &mut Compound) -> ChunkNormalization {
    if root.contains_key("Level") && !root.contains_key("sections") {
        return if is_empty_pre_118_chunk(root) {
            ChunkNormalization::DroppedEmptyPre118
        } else {
            ChunkNormalization::Unchanged
        };
    }

    let mut changed = false;
    if let Some(Tag::List(sections)) = root.get_mut("sections") {
        for sec_tag in sections {
            let Some(Tag::Compound(sec)) = Some(sec_tag) else {
                continue;
            };
            let Some(Tag::Compound(block_states)) = sec.get_mut("block_states") else {
                continue;
            };
            let Some(Tag::List(palette)) = block_states.get_mut("palette") else {
                continue;
            };
            for entry in palette {
                if normalize_palette_entry(entry) {
                    changed = true;
                }
            }
        }
    }

    if changed {
        ChunkNormalization::ConvertedPalette
    } else {
        ChunkNormalization::Unchanged
    }
}

pub fn normalize_region_file(
    path: &Path,
    summary: &mut NormalizationSummary,
) -> Result<(), AnvilCompatError> {
    let raw = std::fs::read(path).map_err(|source| AnvilCompatError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    summary.files_scanned += 1;
    if raw.len() < HEADER_BYTES {
        return Ok(());
    }

    let mut slots: Vec<Option<PackedSlot>> = Vec::with_capacity(ENTRIES);
    let mut file_dirty = false;

    for slot in 0..ENTRIES {
        let loc_off = slot * 4;
        let ts_off = SECTOR_BYTES + slot * 4;
        let loc = u32::from_be_bytes([
            raw[loc_off],
            raw[loc_off + 1],
            raw[loc_off + 2],
            raw[loc_off + 3],
        ]);
        let timestamp = u32::from_be_bytes([
            raw[ts_off],
            raw[ts_off + 1],
            raw[ts_off + 2],
            raw[ts_off + 3],
        ]);
        let sector_offset = ((loc >> 8) & 0x00FF_FFFF) as usize;
        let sector_count = (loc & 0xFF) as usize;
        if sector_offset == 0 || sector_count == 0 {
            slots.push(None);
            continue;
        }

        let byte_offset = sector_offset * SECTOR_BYTES;
        if byte_offset + CHUNK_HEADER_BYTES > raw.len() {
            slots.push(None);
            continue;
        }
        let declared_len = u32::from_be_bytes([
            raw[byte_offset],
            raw[byte_offset + 1],
            raw[byte_offset + 2],
            raw[byte_offset + 3],
        ]) as usize;
        if declared_len == 0 || byte_offset + 4 + declared_len > raw.len() {
            slots.push(None);
            continue;
        }

        let compression_id = raw[byte_offset + 4];
        let payload = &raw[byte_offset + 5..byte_offset + 4 + declared_len];
        let codec = Compression::from_id(compression_id).ok_or_else(|| {
            AnvilCompatError::UnsupportedCompression {
                path: path.to_path_buf(),
                id: compression_id,
            }
        })?;
        let nbt_bytes = codec
            .decompress(payload)
            .map_err(|source| AnvilCompatError::Anvil {
                path: path.to_path_buf(),
                source,
            })?;

        let mut slice = nbt_bytes.as_slice();
        let (root_name, mut root) =
            read_named(&mut slice).map_err(|source| AnvilCompatError::Nbt {
                path: path.to_path_buf(),
                slot,
                source,
            })?;

        match normalize_chunk_nbt(&mut root) {
            ChunkNormalization::Unchanged => {
                summary.chunks_unchanged += 1;
                slots.push(Some(PackedSlot {
                    compression_id,
                    compressed_data: payload.to_vec(),
                    timestamp,
                }));
            }
            ChunkNormalization::DroppedEmptyPre118 => {
                summary.chunks_dropped_empty_legacy += 1;
                file_dirty = true;
                slots.push(None);
            }
            ChunkNormalization::ConvertedPalette => {
                summary.chunks_converted += 1;
                file_dirty = true;
                let mut encoded = Vec::with_capacity(nbt_bytes.len() + 256);
                write_named(&mut encoded, &root_name, &root);
                let compressed_data = Compression::Zlib.compress(&encoded).map_err(|source| {
                    AnvilCompatError::Anvil {
                        path: path.to_path_buf(),
                        source,
                    }
                })?;
                slots.push(Some(PackedSlot {
                    compression_id: Compression::Zlib.id(),
                    compressed_data,
                    timestamp,
                }));
            }
        }
    }

    if !file_dirty {
        return Ok(());
    }

    if slots.iter().all(Option::is_none) {
        std::fs::remove_file(path).map_err(|source| AnvilCompatError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        summary.files_removed_empty += 1;
        return Ok(());
    }

    let mut out = vec![0u8; HEADER_BYTES];
    let mut current_sector: u32 = 2;

    for (slot_idx, slot_opt) in slots.into_iter().enumerate() {
        let Some(packed) = slot_opt else {
            continue;
        };
        let total_chunk_bytes = CHUNK_HEADER_BYTES + packed.compressed_data.len();
        let sector_count = total_chunk_bytes.div_ceil(SECTOR_BYTES);
        let loc = (current_sector << 8) | ((sector_count as u32) & 0xFF);

        let loc_off = slot_idx * 4;
        out[loc_off..loc_off + 4].copy_from_slice(&loc.to_be_bytes());
        let ts_off = SECTOR_BYTES + slot_idx * 4;
        out[ts_off..ts_off + 4].copy_from_slice(&packed.timestamp.to_be_bytes());

        let length_field = (packed.compressed_data.len() + 1) as u32;
        out.extend_from_slice(&length_field.to_be_bytes());
        out.push(packed.compression_id);
        out.extend_from_slice(&packed.compressed_data);

        let padded_bytes = sector_count * SECTOR_BYTES;
        out.resize(out.len() + (padded_bytes - total_chunk_bytes), 0);
        current_sector += sector_count as u32;
    }

    let tmp_path = path.with_extension(format!("mca.{}.tmp", std::process::id()));
    std::fs::write(&tmp_path, &out).map_err(|source| AnvilCompatError::Io {
        path: tmp_path.clone(),
        source,
    })?;
    std::fs::rename(&tmp_path, path).map_err(|source| AnvilCompatError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    summary.files_rewritten += 1;
    Ok(())
}

pub fn normalize_world_dir(world_dir: &Path) -> Result<NormalizationSummary, AnvilCompatError> {
    let mut region_dirs = Vec::new();
    collect_region_dirs(world_dir, &mut region_dirs)?;
    region_dirs.sort();

    let mut summary = NormalizationSummary::default();
    for region_dir in region_dirs {
        let mut mca_files = Vec::new();
        let entries = std::fs::read_dir(&region_dir).map_err(|source| AnvilCompatError::Io {
            path: region_dir.clone(),
            source,
        })?;
        for entry in entries {
            let entry = entry.map_err(|source| AnvilCompatError::Io {
                path: region_dir.clone(),
                source,
            })?;
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("mca") {
                mca_files.push(path);
            }
        }
        mca_files.sort();
        for mca in mca_files {
            normalize_region_file(&mca, &mut summary)?;
        }
    }
    Ok(summary)
}

fn collect_region_dirs(dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), AnvilCompatError> {
    if !dir.is_dir() {
        return Ok(());
    }
    let entries = std::fs::read_dir(dir).map_err(|source| AnvilCompatError::Io {
        path: dir.to_path_buf(),
        source,
    })?;
    for entry in entries {
        let entry = entry.map_err(|source| AnvilCompatError::Io {
            path: dir.to_path_buf(),
            source,
        })?;
        let path = entry.path();
        if path.is_dir() {
            if path.file_name().and_then(|n| n.to_str()) == Some("region") {
                out.push(path);
            } else {
                collect_region_dirs(&path, out)?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_all_three_vanilla_26_palette_variants() {
        let mut bare = Tag::String("minecraft:stone".to_owned());
        assert!(normalize_palette_entry(&mut bare));
        assert_eq!(
            bare,
            Tag::Compound(Compound::new().with("Name", "minecraft:stone"))
        );

        let mut wrapped = Tag::Compound(Compound::new().with("", "minecraft:diamond_block"));
        assert!(normalize_palette_entry(&mut wrapped));
        assert_eq!(
            wrapped,
            Tag::Compound(Compound::new().with("Name", "minecraft:diamond_block"))
        );

        let props = Compound::new()
            .with("facing", "north")
            .with("half", "bottom");
        let mut with_props = Tag::Compound(
            Compound::new()
                .with("id", "minecraft:dark_oak_stairs")
                .with("properties", props.clone()),
        );
        assert!(normalize_palette_entry(&mut with_props));
        assert_eq!(
            with_props,
            Tag::Compound(
                Compound::new()
                    .with("Name", "minecraft:dark_oak_stairs")
                    .with("Properties", props)
            )
        );

        // Idempotent on already normalized tags
        assert!(!normalize_palette_entry(&mut with_props));
    }

    #[test]
    fn marks_all_leaves_blocks_persistent_and_stays_idempotent() {
        let mut bare_leaves = Tag::String("minecraft:oak_leaves".to_owned());
        assert!(normalize_palette_entry(&mut bare_leaves));
        assert_eq!(
            bare_leaves,
            Tag::Compound(
                Compound::new()
                    .with("Name", "minecraft:oak_leaves")
                    .with("Properties", Compound::new().with("persistent", "true"))
            )
        );
        assert!(!normalize_palette_entry(&mut bare_leaves));

        let mut decaying_leaves = Tag::Compound(
            Compound::new()
                .with("Name", "minecraft:cherry_leaves")
                .with(
                    "Properties",
                    Compound::new()
                        .with("distance", "7")
                        .with("persistent", "false"),
                ),
        );
        assert!(normalize_palette_entry(&mut decaying_leaves));
        assert_eq!(
            decaying_leaves,
            Tag::Compound(
                Compound::new()
                    .with("Name", "minecraft:cherry_leaves")
                    .with(
                        "Properties",
                        Compound::new()
                            .with("distance", "7")
                            .with("persistent", "true"),
                    )
            )
        );
        assert!(!normalize_palette_entry(&mut decaying_leaves));
    }

    #[test]
    fn detects_and_drops_empty_pre_118_void_chunks() {
        let mut legacy = Compound::new().with(
            "Level",
            Compound::new().with(
                "Sections",
                Tag::List(vec![Tag::Compound(Compound::new().with("Y", 0i8))]),
            ),
        );
        assert_eq!(
            normalize_chunk_nbt(&mut legacy),
            ChunkNormalization::DroppedEmptyPre118
        );
    }
}
