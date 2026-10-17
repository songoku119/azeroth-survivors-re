//! Whether a BLP's stored S3TC blocks can go to the GPU untouched, as the reference uploads them
//! (`glCompressedTexImage2DARB`; its software DXT decoder is a 16-bit-device fallback, device
//! formats `0x58a230`, per-level upload formats `0x59f270`). They can when the device accepts BC
//! ([`bc_supported`]) and mip 0 is whole 4x4 blocks; otherwise the chain decodes to `Rgba8Unorm`.
//! Both the async loader ([`crate::blp`]) and `WorldAssets` ([`crate::world_assets`]) ask here.

use std::sync::OnceLock;

use benilla_formats::{BlpMipChain, BlpTexels};
use bevy::image::{CompressedImageFormatSupport, CompressedImageFormats};
use bevy::prelude::*;
use bevy::render::render_resource::TextureFormat;

static BC_SUPPORTED: OnceLock<bool> = OnceLock::new();

/// Publish whether this run's GPU accepts BC, once, from a plugin's `finish()`. A `OnceLock`, not
/// a resource, because the async loader has no world access; every `finish()` runs before the
/// first update, so before any BLP loads. Headless, nothing publishes and the decoded path runs.
pub fn publish_bc_support(supported: bool) {
    let _ = BC_SUPPORTED.set(supported);
}

/// Whether this run's GPU accepts BC; `false` until [`publish_bc_support`].
pub fn bc_supported() -> bool {
    *BC_SUPPORTED.get().unwrap_or(&false)
}

/// Publishes [`bc_supported`] from the render device; added by [`crate::register_asset_loaders`].
/// The work is in `finish`: `RenderPlugin::finish` inserts `CompressedImageFormatSupport`, which
/// does not exist at `build` time or in a headless app.
pub struct BlpGpuSupportPlugin;

impl Plugin for BlpGpuSupportPlugin {
    fn build(&self, _app: &mut App) {}

    fn finish(&self, app: &mut App) {
        // `WOW_NO_BC=1` forces the decoded path: the A/B lever, and the first thing to try when a
        // texture looks wrong (hardware block decode is not bit-identical to `texpresso`'s).
        let forced_off = std::env::var("WOW_NO_BC").as_deref() == Ok("1");
        let device_can = app
            .world()
            .get_resource::<CompressedImageFormatSupport>()
            .is_some_and(|s| s.0.contains(CompressedImageFormats::BC));
        let supported = device_can && !forced_off;
        publish_bc_support(supported);
        // At `info`, so a player's log says which texture lane the run is on.
        if supported {
            info!("blp: DXT blocks upload natively (BC)");
        } else if forced_off {
            info!("blp: WOW_NO_BC=1 — decoding textures to RGBA8");
        } else {
            info!("blp: device reports no BC support — decoding textures to RGBA8");
        }
    }
}

fn uploadable_as_blocks(chain: &BlpMipChain) -> bool {
    chain.texels.is_block_compressed()
        && bc_supported()
        && chain.width.is_multiple_of(4)
        && chain.height.is_multiple_of(4)
}

/// A chain and the `TextureFormat` it uploads as, decided together so block bytes can never meet
/// an uncompressed descriptor.
pub struct UploadChain {
    pub chain: BlpMipChain,
    pub format: TextureFormat,
}

/// A BLP's authored mip chain in the best form this GPU takes, with its format: blocks if it can,
/// pixels if not. Always non-sRGB, so the GPU never linearizes albedo on sample.
pub fn for_upload(chain: BlpMipChain) -> UploadChain {
    if uploadable_as_blocks(&chain) {
        let format = match chain.texels {
            BlpTexels::Bc1 => TextureFormat::Bc1RgbaUnorm,
            BlpTexels::Bc2 => TextureFormat::Bc2RgbaUnorm,
            BlpTexels::Bc3 => TextureFormat::Bc3RgbaUnorm,
            // `uploadable_as_blocks` already excluded this arm.
            BlpTexels::Rgba8Unorm => TextureFormat::Rgba8Unorm,
        };
        return UploadChain { chain, format };
    }
    // `into_rgba8` is a no-op on pixels and an infallible decode of blocks. The decoded path is
    // where VRAM goes 4-8× per texture, so the mip drop applies here and nowhere else.
    UploadChain {
        chain: drop_top_mips(chain, mip_drop()).into_rgba8(),
        format: TextureFormat::Rgba8Unorm,
    }
}

/// `WOW_MIP_DROP=n`: how many top levels the decoded path drops from a chain wider than
/// [`MIP_DROP_MIN_WIDTH`], read once like `WOW_NO_BC`. Android defaults to 1, since the RGBA8
/// fallback is the memory gate there and the top-down camera never sees texel detail at 1:1;
/// elsewhere 0, so a desktop `WOW_NO_BC=1 WOW_MIP_DROP=1` run is the A/B.
fn mip_drop() -> u32 {
    static MIP_DROP: OnceLock<u32> = OnceLock::new();
    *MIP_DROP.get_or_init(|| {
        std::env::var("WOW_MIP_DROP")
            .ok()
            .and_then(|v| v.trim().parse().ok())
            .unwrap_or(DEFAULT_MIP_DROP)
    })
}

#[cfg(target_os = "android")]
const DEFAULT_MIP_DROP: u32 = 1;
#[cfg(not(target_os = "android"))]
const DEFAULT_MIP_DROP: u32 = 0;

/// Only chains wider than this are eligible for the drop: 1.12's 256-wide terrain and skin
/// textures stay whole, the 1024-wide building and foliage sheets are the ones that cost.
const MIP_DROP_MIN_WIDTH: u32 = 512;

/// The narrowest a dropped chain's new level 0 may be.
const MIP_DROP_FLOOR_WIDTH: u32 = 256;

/// How many top levels to drop from a `width`-wide chain of `levels` authored levels, asked for
/// `drop`: none unless wider than [`MIP_DROP_MIN_WIDTH`], never below [`MIP_DROP_FLOOR_WIDTH`],
/// always leaving a level.
fn levels_to_drop(width: u32, levels: usize, drop: u32) -> usize {
    if width <= MIP_DROP_MIN_WIDTH || levels == 0 {
        return 0;
    }
    let mut n = 0usize;
    while n < drop as usize && n + 1 < levels && (width >> (n + 1)) >= MIP_DROP_FLOOR_WIDTH {
        n += 1;
    }
    n
}

/// `chain` with its top levels removed per [`levels_to_drop`]; the stored levels are reused as
/// they are, so a block chain stays blocks until `into_rgba8`.
fn drop_top_mips(mut chain: BlpMipChain, drop: u32) -> BlpMipChain {
    let n = levels_to_drop(chain.width, chain.mips.len(), drop);
    if n == 0 {
        return chain;
    }
    let (w, h) = chain.mip_size(n as u32);
    chain.mips.drain(..n);
    chain.width = w;
    chain.height = h;
    chain
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chain(texels: BlpTexels, w: u32, h: u32) -> BlpMipChain {
        let mips = vec![vec![0u8; texels.level_bytes(w, h)]];
        BlpMipChain {
            width: w,
            height: h,
            texels,
            mips,
        }
    }

    #[test]
    fn no_published_support_means_the_decoded_path() {
        // `publish_bc_support` is never called in this test binary, so the lock stays empty.
        assert!(!bc_supported());
        let up = for_upload(chain(BlpTexels::Bc1, 16, 16));
        assert_eq!(up.format, TextureFormat::Rgba8Unorm);
        assert!(up.chain.is_rgba8(), "blocks must have been decoded");
    }

    #[test]
    fn the_returned_format_always_matches_the_returned_bytes() {
        for texels in [
            BlpTexels::Rgba8Unorm,
            BlpTexels::Bc1,
            BlpTexels::Bc2,
            BlpTexels::Bc3,
        ] {
            for (w, h) in [(16u32, 16u32), (8, 32), (258, 256), (6, 6)] {
                let up = for_upload(chain(texels, w, h));
                let expected = match up.format {
                    TextureFormat::Rgba8Unorm => BlpTexels::Rgba8Unorm,
                    TextureFormat::Bc1RgbaUnorm => BlpTexels::Bc1,
                    TextureFormat::Bc2RgbaUnorm => BlpTexels::Bc2,
                    TextureFormat::Bc3RgbaUnorm => BlpTexels::Bc3,
                    other => panic!("unexpected albedo format {other:?}"),
                };
                assert_eq!(up.chain.texels, expected, "{texels:?} at {w}x{h}");
                assert_eq!(
                    up.chain.mips[0].len(),
                    expected.level_bytes(w, h),
                    "mip 0 bytes must match what {:?} implies at {w}x{h}",
                    up.format
                );
            }
        }
    }

    /// wgpu-core rejects a compressed texture that is not whole blocks
    /// (`NotMultipleOfBlockWidth/Height`), and its default error handler panics.
    #[test]
    fn non_block_aligned_dimensions_are_refused() {
        for (w, h) in [(258u32, 256u32), (256, 258), (6, 6), (1, 1)] {
            assert!(
                !uploadable_as_blocks(&chain(BlpTexels::Bc1, w, h)),
                "{w}x{h} is not whole blocks"
            );
        }
    }

    /// A chain with every level down to 1×1, as the 1.12 BLPs author them.
    fn full_chain(texels: BlpTexels, w: u32, h: u32) -> BlpMipChain {
        let levels = 32 - w.max(h).leading_zeros();
        let mips = (0..levels)
            .map(|i| vec![0u8; texels.level_bytes((w >> i).max(1), (h >> i).max(1))])
            .collect();
        BlpMipChain {
            width: w,
            height: h,
            texels,
            mips,
        }
    }

    #[test]
    fn mip_drop_math() {
        // Nothing at or under 512 wide, whatever is asked.
        assert_eq!(levels_to_drop(512, 10, 2), 0);
        assert_eq!(levels_to_drop(256, 9, 1), 0);
        // Zero asked is zero dropped.
        assert_eq!(levels_to_drop(1024, 11, 0), 0);
        // 1024 → 512 at one, 1024 → 256 at two, and the 256 floor holds at three.
        assert_eq!(levels_to_drop(1024, 11, 1), 1);
        assert_eq!(levels_to_drop(1024, 11, 2), 2);
        assert_eq!(levels_to_drop(1024, 11, 3), 2);
        // 2048 → 256 is three; 9 asked still stops at the floor.
        assert_eq!(levels_to_drop(2048, 12, 9), 3);
        // A chain authored without mips has nothing to drop; one spare level allows one drop.
        assert_eq!(levels_to_drop(1024, 1, 1), 0);
        assert_eq!(levels_to_drop(1024, 2, 2), 1);
        assert_eq!(levels_to_drop(1024, 0, 2), 0);
    }

    #[test]
    fn dropped_chain_keeps_dimensions_and_bytes_in_step() {
        for texels in [BlpTexels::Bc1, BlpTexels::Bc3, BlpTexels::Rgba8Unorm] {
            let before = full_chain(texels, 1024, 512);
            let levels = before.mips.len();
            let c = drop_top_mips(before, 1);
            assert_eq!((c.width, c.height), (512, 256), "{texels:?}");
            assert_eq!(c.mips.len(), levels - 1);
            for (i, level) in c.mips.iter().enumerate() {
                let (w, h) = c.mip_size(i as u32);
                assert_eq!(level.len(), texels.level_bytes(w, h), "{texels:?} level {i}");
            }
            // The decode after the drop still agrees with the new level 0.
            let decoded = c.into_rgba8();
            assert_eq!(
                decoded.mips[0].len(),
                BlpTexels::Rgba8Unorm.level_bytes(512, 256)
            );
        }
        // Ineligible chains come back untouched.
        let c = drop_top_mips(full_chain(BlpTexels::Bc1, 512, 512), 2);
        assert_eq!((c.width, c.height, c.mips.len()), (512, 512, 10));
    }

    #[test]
    fn an_rgba_chain_passes_through_untouched() {
        let c = chain(BlpTexels::Rgba8Unorm, 8, 8);
        let before = c.mips[0].len();
        let up = for_upload(c);
        assert_eq!(up.format, TextureFormat::Rgba8Unorm);
        assert_eq!(up.chain.mips[0].len(), before);
    }
}
