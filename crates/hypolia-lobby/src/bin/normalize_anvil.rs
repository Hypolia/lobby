use std::path::PathBuf;

use hypolia_lobby::anvil_compat::normalize_world_dir;
use rspigot_anvil::{AnvilStore, DimensionLayout};
use rspigot_core::ports::ChunkStore;
use rspigot_domain::{BlockPos, BlockStateId, ChunkPos, Identifier};

fn main() {
    let world_dir = std::env::args()
        .nth(1)
        .map_or_else(|| PathBuf::from("./server/world"), PathBuf::from);

    println!(
        "Normalisation des régions Anvil 26.x pour rspigot dans {} ...",
        world_dir.display()
    );

    let summary = match normalize_world_dir(&world_dir) {
        Ok(s) => s,
        Err(err) => {
            eprintln!("Erreur lors de la normalisation : {err}");
            std::process::exit(1);
        }
    };

    println!(
        "Terminé : {} fichiers scannés, {} réécrits, {} régions vides supprimées | {} chunks convertis, {} inchangés, {} chunks vides pré-1.18 purgés.",
        summary.files_scanned,
        summary.files_rewritten,
        summary.files_removed_empty,
        summary.chunks_converted,
        summary.chunks_unchanged,
        summary.chunks_dropped_empty_legacy
    );

    // Vérification directe via AnvilStore de rspigot
    match AnvilStore::open(&world_dir, DimensionLayout::Spigot) {
        Ok(store) => {
            let overworld = Identifier::minecraft("overworld");
            if let Ok(Some(col)) = store.load(&overworld, ChunkPos::new(0, 0)) {
                let mut found = Vec::new();
                for y in 40..=90 {
                    let pos = BlockPos::new(0, y, 8);
                    if let Some(state) = col.block(pos)
                        && state != BlockStateId::AIR
                    {
                        found.push((y, state));
                    }
                }
                println!(
                    "Vérification rspigot Chunk(0,0) sur (X=0, Z=8) : {} blocs non-air détectés -> {:?}",
                    found.len(),
                    found
                );
            }
        }
        Err(err) => {
            eprintln!("Avertissement : vérification AnvilStore ignorée ({err})");
        }
    }
}
