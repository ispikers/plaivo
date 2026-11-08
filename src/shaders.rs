use std::path::PathBuf;

pub struct Preset {
    pub id: &'static str,
    pub name: &'static str,
    pub files: &'static [&'static str],
}

const CLAMP: &str = "Anime4K_Clamp_Highlights.glsl";
const RESTORE_M: &str = "Anime4K_Restore_CNN_M.glsl";
const RESTORE_VL: &str = "Anime4K_Restore_CNN_VL.glsl";
const UPSCALE_M: &str = "Anime4K_Upscale_CNN_x2_M.glsl";
const UPSCALE_VL: &str = "Anime4K_Upscale_CNN_x2_VL.glsl";
const DOWNSCALE: &str = "Anime4K_AutoDownscalePre_x2.glsl";

pub const NVSCALER: &str = "nvscaler";

pub const PRESETS: &[Preset] = &[
    Preset { id: NVSCALER, name: "NVScaler", files: &["NVScaler.glsl"] },
    Preset { id: "artcnn_c4f16", name: "ArtCNN C4F16", files: &["ArtCNN_C4F16.glsl"] },
    Preset { id: "artcnn_c4f16_dn", name: "ArtCNN C4F16 Denoise", files: &["ArtCNN_C4F16_DN.glsl"] },
    Preset { id: "artcnn_c4f16_ds", name: "ArtCNN C4F16 Denoise + Sharpen", files: &["ArtCNN_C4F16_DS.glsl"] },
    Preset { id: "artcnn_c4f32", name: "ArtCNN C4F32", files: &["ArtCNN_C4F32.glsl"] },
    Preset { id: "artcnn_c4f32_dn", name: "ArtCNN C4F32 Denoise", files: &["ArtCNN_C4F32_DN.glsl"] },
    Preset { id: "artcnn_c4f32_ds", name: "ArtCNN C4F32 Denoise + Sharpen", files: &["ArtCNN_C4F32_DS.glsl"] },
    Preset { id: "anime4k_fast_a", name: "Anime4K Fast A", files: &[CLAMP, RESTORE_M] },
    Preset { id: "anime4k_fast_b", name: "Anime4K Fast B", files: &[CLAMP, RESTORE_M, UPSCALE_M, DOWNSCALE] },
    Preset { id: "anime4k_fast_c", name: "Anime4K Fast C", files: &[CLAMP, UPSCALE_M, DOWNSCALE] },
    Preset { id: "anime4k_fast_aa", name: "Anime4K Fast A+A", files: &[CLAMP, RESTORE_M, RESTORE_M] },
    Preset { id: "anime4k_fast_bb", name: "Anime4K Fast B+B", files: &[CLAMP, RESTORE_M, RESTORE_M, UPSCALE_M, DOWNSCALE] },
    Preset { id: "anime4k_fast_ca", name: "Anime4K Fast C+A", files: &[CLAMP, UPSCALE_M, RESTORE_M, DOWNSCALE] },
    Preset { id: "anime4k_hq_a", name: "Anime4K HQ A", files: &[CLAMP, RESTORE_VL] },
    Preset { id: "anime4k_hq_b", name: "Anime4K HQ B", files: &[CLAMP, RESTORE_VL, UPSCALE_VL, DOWNSCALE] },
    Preset { id: "anime4k_hq_c", name: "Anime4K HQ C", files: &[CLAMP, UPSCALE_VL, DOWNSCALE] },
    Preset { id: "anime4k_hq_aa", name: "Anime4K HQ A+A", files: &[CLAMP, RESTORE_VL, RESTORE_VL] },
    Preset { id: "anime4k_hq_bb", name: "Anime4K HQ B+B", files: &[CLAMP, RESTORE_VL, RESTORE_VL, UPSCALE_VL, DOWNSCALE] },
    Preset { id: "anime4k_hq_ca", name: "Anime4K HQ C+A", files: &[CLAMP, UPSCALE_VL, RESTORE_VL, DOWNSCALE] },
];

pub fn find(id: &str) -> Option<&'static Preset> {
    PRESETS.iter().find(|p| p.id == id)
}

pub fn name(id: &str) -> &'static str {
    find(id).map_or("Off", |p| p.name)
}

pub fn dir() -> PathBuf {
    std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.join("shaders"))).unwrap_or_else(|| "shaders".into())
}

pub fn is_hdr(primaries: &str, gamma: &str) -> bool {
    primaries.contains("bt.2020") || matches!(gamma, "pq" | "hlg")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_preset_ships_its_files_once() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("shaders");
        let mut ids = std::collections::HashSet::new();
        for p in PRESETS {
            assert!(ids.insert(p.id), "duplicate {}", p.id);
            for f in p.files {
                assert!(root.join(f).is_file(), "{} needs shaders/{f}", p.id);
            }
        }
        assert_eq!(name(""), "Off");
        assert_eq!(name(NVSCALER), "NVScaler");
    }

    #[test]
    fn hdr_is_detected_from_primaries_or_transfer() {
        assert!(is_hdr("bt.2020", "pq"));
        assert!(is_hdr("bt.709", "hlg"));
        assert!(is_hdr("bt.2020", "bt.1886"));
        assert!(!is_hdr("bt.709", "bt.1886"));
        assert!(!is_hdr("", ""));
    }
}
