//! The twelve reference frames of M6 task 3: the Enhanced picture at 4x, drawn by `--ui-snapshot`
//! from the reference replays (`.work/replays`) and written to `.work/hd/` (untracked).
//!
//! Each picture is compared with a signature, not with its bytes: the GPU rounds the texture
//! filtering differently from one card to another (a channel off by one or two on a fraction of a
//! percent of the pixels), so the bytes of the file differ while the picture does not. The
//! signature is coarse on purpose. Per cell of 128 x 128 pixels it keeps the mean of each channel
//! and the mean edge strength; for the whole picture, the centre of the edge strength. A cell
//! mean is an average of 16384 pixels, and the centre a ratio of sums over the whole picture: the
//! signature cannot be turned back into the picture, and it holds no pixel of the game.
//!
//! A value may move by one bucket (one level of a channel, two levels of edge strength, a quarter
//! of a pixel of centre) and still match. GPU rounding moves a value by a few hundredths of a
//! bucket, so it always matches; a real change moves values by many buckets (a blanked text cell,
//! a recoloured window, a picture shifted by one pixel, a filter) and always fails.
//!
//! Needs `SSD_ROM_PATH` (the ROM) and the replays; run with
//! `cargo test -p ssd-app --release --test hd_snapshots -- --ignored`. The signature checks on
//! synthetic pictures run with the other tests of the crate.

use std::collections::HashMap;
use std::path::Path;
use std::process::Command;

/// The frames: the name of the PNG, the replay, the frames played before the picture, and the
/// signature of the picture (see [`encode`]).
const FRAMES: [(&str, &str, u64, &str); 12] = [
    (
        "01-title",
        "landing",
        1600,
        "049203bc0000ff002323b60149498002393993013c3c89023a3a8d022f1ba10152527e010202e200\
        0000fe000000ff001a1ab4016868350379792d0454545003645c2f047c240d03645f36020000fe00\
        0000ff000000ff002e009100a700020188000f018600050114008000a1000d01350072000000ff00\
        0000ff000000ff001111f6023916b304211eda04170ddc0218009c00860023012c007a000000ff00\
        0000ff000000ff002021f6019588c908857bd1079288d407877dcd08988ace082d2af2020000ff00\
        0000ff000000ff00272ffe02b8b7d80899a1ea06919ce905a5abe207b0acd2084149fa040000ff00\
        0000ff000000ff000000ff001818ff043232ff073a3aff072e2eff070909ff010000ff000000ff00\
        0000ff000000ff000000ff000000ff000000ff000000ff000000ff000000ff000000ff000000ff00\
        0000ff0008f2086b",
    ),
    (
        "02-flight",
        "landing",
        500,
        "049203bc0000ff000000ff000000ff000000ff000000ff000000ff000000ff000000ff000000fa00\
        0000fe000000ff000000ff000000ff000000ff000000ff000000ff000f0fff000000ff000000ff00\
        0000ff000000ff000000ff000000ff000000ff000000ff000405ff007686ff020000ff000000ff00\
        0000ff000000ff000000ff000000ff000e10ff010000ff000000ff000000ff000000ff000000ff00\
        0000ff000000ff000000ff000000ff000000ff000000ff000000ff000000ff000000ff000000ff00\
        0000ff000000ff000000ff002525ff010000ff000000ff000000ff000000ff000000ff000000ff00\
        0000ff000000ff00272bff01b7ceff021315ff010000ff000000ff000000ff000000ff000000ff00\
        0000ff000000ff000000ff000001ff000000ff000000ff000000ff000000ff000000ff000000ff00\
        0000ff00081c0902",
    ),
    (
        "03-field",
        "story-1",
        9500,
        "049203bc66b6fe0064b5fe0066b6fe0065b6fe0066b6fe0067b6fe0165b6fe0066b6fe0064b3f900\
        68b7fd0166b6fe0066b6fe0063b5fe0067b7fe0166b6fe0066b6fe0066b6fe0065b6fe0064b5fe00\
        63b5ff0066b6fe0066b6fe0074bcec0170bfa80368b76f056dbd98047abed30271bafa0166b6fe00\
        67b7fe0165b6fe0066b6fe0086bedc0360ac4c075d8b490b4d92430773b84d0683b7ab0466b6fe00\
        68b7fe0164b5fe0065b6fe006ab8fe0175ba7106409b330766ab430687ae920573b7f10164b5fe00\
        64b5ff0066b6fe0066b6fe006ab8fe018cbc9d0572ab480681af960569b8fe0165b6fe0066b6fe00\
        68b7fe0166b6fe0064b5fe0066b6fe006bb8fe0174b9f5026db9fd0165b6fe0064b5fe0066b6fe00\
        68b7fe0166b6fe0064b5fe0064b5fe0067b7fe0164b5fe0066b6fe0063b5ff0066b6fe0064b5fe00\
        63b5ff00098e07c1",
    ),
    (
        "04-message",
        "landing",
        2400,
        "049203bc0000840003006d000600270003000300000000000300030008001e000300640000008200\
        000083000000840003006d000900290003000300000000000300030006001f000500640000008400\
        000084000000840003006d0009002b0002000200060301000400030004001d000600640000008400\
        000084000000840005006e000700290000000000270f05030100010003001d000500650000008400\
        0000840006068601060270000b062d01050505010e07050208080801090622010b06690107078701\
        060687013e3ea20934308a083c36550934343409222222063b3b3b0a2d294106383585091c1c9104\
        2222940531319c074541930a2d29490635353508303030083b3a3b0929223d061817730417178f03\
        13138d032d2d99063c398f092c294706262626074545450a2623260739334c083d3c880905058600\
        00008400086e0b96",
    ),
    (
        "05-command-menu",
        "story-1",
        9900,
        "049203bc5f98cd035f89b00566b6fe0066b6fe0064b5fe0065b6fe0064b5fe0068a9d106609ec306\
        63b4fe006fa3d8037597be0464b5fe0066b6fe0065b6fe0066b6fe0065b6fe0068abd40562a6d105\
        63b5fe0065b6fe0065b6fe0065b6fe0065b6fe006ab8fe016bb8fe0166b6fe0064b5fe0065b6fe00\
        63b5ff0066b6fe0066b6fe006cb8fd0077bfbd03608f5a085eb95e0577beb6037bbeda0268b7fe01\
        63b5fe0081acdc02a4b1c404a1acc2048dbe970542a5370653855c0869b64a066aab560679bffd01\
        63b5fe009dadc702e5d6bc06dbc5b30288bec80445aa37064aae39068aaa730682b8d70364b5fe00\
        63b5ff009fb3cd02e3d5bc06d6c1b1037fbce70284b46d067dac5c0778baf30165b6fe0067b6fe00\
        63b5fe007ba2d4029d9cb1039197b30366b6fe006fbafd0176b9f60265b6fe0066b6fe0064b5fe00\
        63b5ff00090d0833",
    ),
    (
        "06-menu-list",
        "secret-namek-zfighters",
        2700,
        "049203bc294b570534464b053b5144041845590303436d030a4569041244610450615f0537566805\
        1e4a5904795c2b099171470b5a3e0405343623034a4d2b055f4104044b421a056549250a62471e08\
        5b4004026b572e0a776141093e2502044128020453380406684a05043e2601046f4f16073a1e0b06\
        06000000422e1d084f3b2e08452e0305432701043922070538200205563c04053720010472520504\
        4f3200035439260682624f0694674907bd927307b8977f0690746104342a140627180705321e0604\
        0b101402a5876004e6d8be05ecb89603e7d5bb05e2d4bc07ab9f940202447003164964042a4c5604\
        5e5f4405646c7605e2d3ba07e7b59404e7d4bb06efe0c404b1a2930206456d030144700306497403\
        125b83032a466204675e64036453580464585f03675f6503474e5f033f5450041d4b6104265e7503\
        1e71990108890709",
    ),
    (
        "07-menu-status",
        "secret-namek-zfighters",
        3000,
        "049203bc301d1602755c4d056a4f40052d241d05745b4c0567554704634b3d066a55430530221603\
        0000000061483903ebdcc104d8cab209ceb89b09dbceb809d6bc9b08d0bea30ad9c5a2086f543a04\
        0000000059403405d5c7b40abfb3a40d93826f08cdc0af0caca69f0cc3b8aa0892887c083b2e2602\
        0000000061483903ebdcc104d2c4ad09c5a88809d9cbb709c4b5ad09e3d7c402baaf960048392f00\
        0000000059403405d9cbb609c8bca9096c655904d4c6b30a7a726505d1c3aa02aea38d0140322900\
        0000000061483903f4e4c702eedfc000857c6c02ffefce00988e7b01dbcdb101baaf960048392f00\
        000000003e261d03a1857104af937a00ad907801af937a00af937a00ad917901af937a005e423301\
        00000000000000000000000000000000000000000000000000000000000000000000000000000000\
        0000000008470582",
    ),
    (
        "08-battle-command",
        "story-1",
        8400,
        "049203bc5e97db02a3b3ca0391a4c3027690b3045575990796abc9038fa8c90292abca02789ed003\
        2d96fe0089a1c702e4d4bb06ebcdae04c8ae9709ae867b0ce6d7be06e0d2bb07faeacb019aacc601\
        2994ff005e99df028ca9ce0198aac70282a0c90182a2cc018fa5c6047d9ec903a4b5ce026297d801\
        2994ff002994ff003a9dff012994ff002994ff003698fa0149a4ff013198ff002994ff0042a0ff01\
        2d96ff00529fe302639bd7035b98d7033cabfe013f89c20b62b2f60354b5ff023cabff015faee502\
        69a3ce019f867204e9c7a905d3ac95035f7e95043f75a2035a7f98058a978006879c830586916e03\
        7a8a6e0176616503e4d5bc06ccad98034f464107564b42076d534f0972532e0b7154310c574b4107\
        2131520043374a017854540168474c013a415304454650044a49510653515507524f530743445004\
        21315200096d084a",
    ),
    (
        "09-attack",
        "story-1",
        8900,
        "049203bcf8730005f27d0006f67e0005f5890004fa8f0003f6870004f7800005f17d0006f3730005\
        fa700003f88a0003f97e0005f4720009f071000af27d0006f0720009f3700009f97c0005f8890003\
        f8870003f4850004f4850005f27c0007e662010dab630c0cec68000cf27a0007f4840005f4840004\
        f2870003f4840004f3860005f27c0007eb62000dd075050add62030df2790007f3850005f4850004\
        f2800004f88a0003f97c0005f3720009f1740009f37f0006f1750009f2710009f97a0005f8880003\
        f89000025a3f36038d563408e6b07e06eab58005eab58005e6b27e06edb78205a47a610868595705\
        5140400036384e027f5a5206ffefce00ffefce00ffefce00ffefce00ffefce007c71790521375b03\
        213152002131520036324900785454017854540178545401785454017854540143374a0021315200\
        2131520009170683",
    ),
    (
        "10-ending-story",
        "secret-ending-normal",
        6000,
        "049203bc0803020034280c02282a09022d290902282a0902342b0e02282a0902282a09020f070300\
        000000001f0d060182e5250255ec20015cd4160462b924066ee8240273ee250250ee1c012e270b01\
        000000001f0d06019cdca6039de2ab038383810ba0b59e07bbd08b03b2cf700397bf9f0436261701\
        000000001f0d060195bbd3039bb7c50278b7e6029dc4d602c9bba504bbac980380c3f20132232001\
        000000001e0d06016888ae067e96af055a8bbc064a86c4067e96af055e8cbb06668fb7062c1b1901\
        00000000000000000b0b0b030b0b0b020b0b0b030303030000000000000000000000000000000000\
        00000000000000000000000000000000000000000000000000000000000000000000000000000000\
        00000000000000000000000000000000000000000000000000000000000000000000000000000000\
        000000000895061c",
    ),
    (
        "11-ending-credits",
        "secret-ending-normal",
        9000,
        "049203bc0803020028270802282a09022c2a0b02312b0c02282a09022c290902282a09020f070300\
        000000001f0d060164e41e0158ec200188d422056ab9230654e8200155ec270169ee220130270c01\
        000000001f0d06018dcda504a7c8ad04a88e5e0aa4a55f07aae3ad03bbe1ac02ade0bc0337251d01\
        000000001f0d06018fbddd0299c3d802cebba004c1af970386c2e8017ec5f501a8babd0336201801\
        000000001e0d06016486b0064e87c2064a86c3065589bf06618dba065589be068097ae05311c1701\
        0000000000000000060b00011d1d1d051b1b1b04161616031e1e1e051e1e1e050a0f040200000000\
        0000000000000000000000001f293303242f3c04161c2402242e3a04161d24020000000000000000\
        00000000000000000000000000000000000000000000000000000000000000000000000000000000\
        0000000008b9072e",
    ),
    (
        "12-ending-end",
        "secret-ending-normal",
        15000,
        "049203bc000000000000000000000000000000000000000000000000000000000000000000000000\
        00000000000000000000000000000000000000000000000000000000000000000000000000000000\
        000000000a0a0a004747470047474700323232010c0c0c0033333301474747013535350102020200\
        000000000f1e210066c1d901538b9a02696d580890754c0a7a795c0963bbd1004b737d012d677500\
        0000000014181900899ea3017f7f7f016f6a6603473f38036e665d0392a09c0275858a021a2a2e00\
        00000000000000000000000000000000000000000000000000000000000000000000000000000000\
        00000000000000000000000000000000000000000000000000000000000000000000000000000000\
        00000000000000000000000000000000000000000000000000000000000000000000000000000000\
        00000000092a076a",
    ),
];

/// The side of a cell of the signature, in pixels of the picture: 128 pixels, 32 game pixels at 4x.
const CELL: usize = 128;
/// The edge strength of a cell is kept in steps of this many levels, so that it fits a byte.
const EDGE_STEP: u64 = 2;
/// The smallest luma step that counts as an edge. GPU rounding moves a channel by one or two
/// levels, so the steps of a flat area move by five levels at most: below eight they are noise.
const EDGE_FLOOR: u32 = 8;
/// The centre of the edge strength is kept in steps of a quarter of a pixel.
const CENTRE_STEPS_PER_PIXEL: u64 = 4;
/// How far a value may move, in buckets, and still match.
const TOLERANCE: u16 = 1;

/// The signature of one picture: its size, per cell (row by row) the mean of red, green and blue
/// and the mean edge strength, and the centre of the edge strength over the whole picture.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Signature {
    width: u32,
    height: u32,
    cells: Vec<[u8; 4]>,
    centre: [u16; 2],
}

/// The luma of an RGBA pixel, in levels (0 to 255).
fn luma(pixel: &[u8]) -> u32 {
    (77 * u32::from(pixel[0]) + 150 * u32::from(pixel[1]) + 29 * u32::from(pixel[2])) >> 8
}

/// The luma step between two neighbouring pixels, if it is an edge: at least `EDGE_FLOOR` levels.
fn step(a: u32, b: u32) -> u32 {
    let difference = a.abs_diff(b);
    if difference >= EDGE_FLOOR { difference } else { 0 }
}

/// The signature of `rgba` (`width` x `height` pixels, top row first). The edge strength of a
/// pixel is the step to its right plus the step below it (see [`step`]); the last column and row
/// have none.
fn signature(width: u32, height: u32, rgba: &[u8]) -> Signature {
    let (w, h) = (width as usize, height as usize);
    let luma: Vec<u32> = rgba.as_chunks::<4>().0.iter().map(|p| luma(p)).collect();
    let mut edge = vec![0_u32; w * h];
    for y in 0..h {
        for x in 0..w {
            let i = y * w + x;
            let right = if x + 1 < w { step(luma[i], luma[i + 1]) } else { 0 };
            let below = if y + 1 < h { step(luma[i], luma[i + w]) } else { 0 };
            edge[i] = right + below;
        }
    }
    let mut cells = Vec::new();
    for y0 in (0..h).step_by(CELL) {
        for x0 in (0..w).step_by(CELL) {
            let (x1, y1) = ((x0 + CELL).min(w), (y0 + CELL).min(h));
            let mut sums = [0_u64; 4];
            for y in y0..y1 {
                for x in x0..x1 {
                    let i = y * w + x;
                    for (sum, &channel) in sums.iter_mut().zip(&rgba[4 * i..4 * i + 3]) {
                        *sum += u64::from(channel);
                    }
                    sums[3] += u64::from(edge[i]);
                }
            }
            let count = ((x1 - x0) * (y1 - y0)) as u64;
            cells.push([
                bucket(sums[0], count, 1),
                bucket(sums[1], count, 1),
                bucket(sums[2], count, 1),
                bucket(sums[3], count, EDGE_STEP),
            ]);
        }
    }
    let (mut total, mut moment_x, mut moment_y) = (0_u64, 0_u64, 0_u64);
    for (i, &strength) in edge.iter().enumerate() {
        let strength = u64::from(strength);
        total += strength;
        moment_x += strength * (i % w) as u64;
        moment_y += strength * (i / w) as u64;
    }
    let centre = if total == 0 {
        [u16::MAX; 2]
    } else {
        [moment_x, moment_y].map(|moment| {
            u16::try_from(CENTRE_STEPS_PER_PIXEL * moment / total).unwrap_or(u16::MAX)
        })
    };
    Signature { width, height, cells, centre }
}

/// `sum / count` in steps of `step` levels, saturated to a byte.
fn bucket(sum: u64, count: u64, step: u64) -> u8 {
    u8::try_from(sum / (count * step)).unwrap_or(u8::MAX)
}

/// The values of `actual` that are beyond [`TOLERANCE`] from `expected`, as messages (empty when
/// the pictures match). A picture of another size is one message.
fn differences(actual: &Signature, expected: &Signature) -> Vec<String> {
    if (actual.width, actual.height) != (expected.width, expected.height) {
        return vec![format!(
            "the picture is {}x{}, expected {}x{}",
            actual.width, actual.height, expected.width, expected.height
        )];
    }
    let cols = (expected.width as usize).div_ceil(CELL);
    let names = ["red", "green", "blue", "edge"];
    let mut out = Vec::new();
    for (i, (a, e)) in actual.cells.iter().zip(&expected.cells).enumerate() {
        for k in 0..4 {
            if u16::from(a[k].abs_diff(e[k])) > TOLERANCE {
                out.push(format!(
                    "cell {},{} {}: {} expected {}",
                    i % cols,
                    i / cols,
                    names[k],
                    a[k],
                    e[k]
                ));
            }
        }
    }
    for (k, axis) in ["x", "y"].iter().enumerate() {
        if actual.centre[k].abs_diff(expected.centre[k]) > TOLERANCE {
            out.push(format!(
                "the centre of the edges {axis}: {} expected {} (quarter pixels)",
                actual.centre[k], expected.centre[k]
            ));
        }
    }
    out
}

/// Whether the pictures differ beyond the tolerance.
fn differs(actual: &Signature, expected: &Signature) -> bool {
    !differences(actual, expected).is_empty()
}

/// The signature as hex text: the size (two 16-bit numbers), then per cell its four bytes, then
/// the centre (two 16-bit numbers), all big-endian. The entries of [`FRAMES`] are written this way.
fn encode(signature: &Signature) -> String {
    let mut bytes = Vec::new();
    bytes.extend(u16::try_from(signature.width).unwrap().to_be_bytes());
    bytes.extend(u16::try_from(signature.height).unwrap().to_be_bytes());
    for cell in &signature.cells {
        bytes.extend(cell);
    }
    for value in signature.centre {
        bytes.extend(value.to_be_bytes());
    }
    bytes.iter().map(|byte| format!("{byte:02x}")).collect::<Vec<_>>().concat()
}

/// The signature that [`encode`] wrote, read back.
fn decode(text: &str) -> Signature {
    let bytes: Vec<u8> = (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).expect("hex"))
        .collect();
    let width = u32::from(u16::from_be_bytes([bytes[0], bytes[1]]));
    let height = u32::from(u16::from_be_bytes([bytes[2], bytes[3]]));
    let cells_count = (width as usize).div_ceil(CELL) * (height as usize).div_ceil(CELL);
    let body = &bytes[4..4 + 4 * cells_count];
    let cells = body.as_chunks::<4>().0.to_vec();
    let tail = &bytes[4 + 4 * cells_count..];
    let centre = [u16::from_be_bytes([tail[0], tail[1]]), u16::from_be_bytes([tail[2], tail[3]])];
    Signature { width, height, cells, centre }
}

/// The largest difference of the signatures, in buckets (zero for pictures that agree).
fn deviation(actual: &Signature, expected: &Signature) -> u16 {
    if (actual.width, actual.height) != (expected.width, expected.height) {
        return u16::MAX;
    }
    let cells = actual
        .cells
        .iter()
        .zip(&expected.cells)
        .flat_map(|(a, e)| (0..4).map(|k| u16::from(a[k].abs_diff(e[k]))));
    let centre = (0..2).map(|k| actual.centre[k].abs_diff(expected.centre[k]));
    cells.chain(centre).max().unwrap_or(0)
}

/// Deterministic pseudo-random numbers (xorshift64): every run draws the same noise.
struct Draws(u64);

impl Draws {
    fn draw(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
}

/// `rgba` with a colour channel off by one or two on `per_mille` pixels in a thousand: what a
/// different GPU's rounding does to the picture.
fn rounded_differently(rgba: &[u8], seed: u64, per_mille: u64) -> Vec<u8> {
    let mut draws = Draws(seed);
    let mut out = rgba.to_vec();
    for pixel in out.as_chunks_mut::<4>().0 {
        if draws.draw() % 1000 < per_mille {
            for channel in &mut pixel[..3] {
                let step = i8::try_from(1 + draws.draw() % 2).unwrap_or(1);
                let sign: i8 = if draws.draw().is_multiple_of(2) { 1 } else { -1 };
                *channel = channel.saturating_add_signed(sign * step);
            }
        }
    }
    out
}

/// `rgba` moved one pixel to the right, the first column repeated.
fn shifted_right(rgba: &[u8], width: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(rgba.len());
    for row in rgba.chunks_exact(4 * width) {
        out.extend_from_slice(&row[..4]);
        out.extend_from_slice(&row[..row.len() - 4]);
    }
    out
}

/// `rgba` with the square of `side` pixels at (`x0`, `y0`) painted in its most common colour.
fn blanked(rgba: &[u8], width: usize, (x0, y0): (usize, usize), side: usize) -> Vec<u8> {
    let mut colours: HashMap<[u8; 4], usize> = HashMap::new();
    for y in y0..y0 + side {
        for x in x0..x0 + side {
            let i = 4 * (y * width + x);
            *colours.entry(rgba[i..i + 4].try_into().unwrap()).or_default() += 1;
        }
    }
    let background = colours.into_iter().max_by_key(|&(_, n)| n).map(|(c, _)| c).unwrap_or([0; 4]);
    let mut out = rgba.to_vec();
    for y in y0..y0 + side {
        for x in x0..x0 + side {
            let i = 4 * (y * width + x);
            out[i..i + 4].copy_from_slice(&background);
        }
    }
    out
}

/// `rgba` with 24 levels more red in the rectangle from (`x0`, `y0`) to (`x1`, `y1`).
fn recoloured(
    rgba: &[u8],
    width: usize,
    (x0, y0): (usize, usize),
    (x1, y1): (usize, usize),
) -> Vec<u8> {
    let mut out = rgba.to_vec();
    for y in y0..y1 {
        for x in x0..x1 {
            let i = 4 * (y * width + x);
            out[i] = out[i].saturating_add(24);
        }
    }
    out
}

/// `rgba` with every colour channel averaged over the 3 by 3 pixels around it.
fn blurred(rgba: &[u8], width: usize, height: usize) -> Vec<u8> {
    let mut out = rgba.to_vec();
    for y in 0..height {
        for x in 0..width {
            for channel in 0..3 {
                let (mut sum, mut count) = (0_u32, 0_u32);
                for ny in y.saturating_sub(1)..(y + 2).min(height) {
                    for nx in x.saturating_sub(1)..(x + 2).min(width) {
                        sum += u32::from(rgba[4 * (ny * width + nx) + channel]);
                        count += 1;
                    }
                }
                out[4 * (y * width + x) + channel] = u8::try_from(sum / count).unwrap_or(u8::MAX);
            }
        }
    }
    out
}

/// The 32 x 32 square (at a multiple of 32) with the largest luma change: a text cell of the picture.
fn busiest_square(rgba: &[u8], width: usize, height: usize) -> (usize, usize) {
    let mut best = ((0, 0), 0);
    for y0 in (0..height.saturating_sub(32)).step_by(32) {
        for x0 in (0..width.saturating_sub(32)).step_by(32) {
            let mut edges = 0;
            for y in y0..y0 + 32 {
                for x in x0..x0 + 31 {
                    let i = 4 * (y * width + x);
                    edges += luma(&rgba[i..i + 4]).abs_diff(luma(&rgba[i + 4..i + 8]));
                }
            }
            if edges > best.1 {
                best = ((x0, y0), edges);
            }
        }
    }
    best.0
}

/// The checks of one rendered picture: the rounding of a GPU must match, and each real change
/// must not. Returns what went wrong.
fn check_rendered(
    name: &str,
    width: u32,
    height: u32,
    rgba: &[u8],
    expected: &Signature,
) -> Vec<String> {
    let (w, h) = (width as usize, height as usize);
    let mut problems = Vec::new();
    let actual = signature(width, height, rgba);
    if deviation(&actual, expected) > TOLERANCE {
        problems.push(format!("{name}: {}", differences(&actual, expected).join("; ")));
    }
    let changes = [
        ("1% of the pixels off by one or two", rounded_differently(rgba, 7, 10), true),
        ("shifted by one pixel", shifted_right(rgba, w), false),
        ("a text cell blanked", blanked(rgba, w, busiest_square(rgba, w, h), 32), false),
        ("a window recoloured", recoloured(rgba, w, (w / 4, h / 4), (w / 2, h / 2)), false),
        ("a filter applied", blurred(rgba, w, h), false),
    ];
    for (what, changed, must_match) in changes {
        let differences = differences(&signature(width, height, &changed), expected);
        if differences.is_empty() != must_match {
            problems.push(format!(
                "{name}: {what} gives {}",
                if differences.is_empty() { "a match" } else { "a mismatch" }
            ));
        }
    }
    problems
}

#[test]
#[ignore = "needs SSD_ROM_PATH and the reference replays"]
fn the_twelve_reference_frames_at_four_times_match_their_signatures() {
    let rom = std::env::var("SSD_ROM_PATH").unwrap();
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..");
    let replays = root.join(".work").join("replays");
    let out_dir = root.join(".work").join("hd");
    std::fs::create_dir_all(&out_dir).unwrap();
    let data = tempfile::tempdir().unwrap();
    let mut problems = Vec::new();
    for (name, replay, frames, reference) in FRAMES {
        let out = out_dir.join(format!("{name}.png"));
        let run = Command::new(env!("CARGO_BIN_EXE_ssd-recompiled"))
            .args(["--rom", &rom, "--replay"])
            .arg(replays.join(format!("{replay}.ssdreplay")))
            .args(["--headless-frames", &frames.to_string()])
            .args(["--hd-scale", "4", "--no-hd-font", "--ui-size", "1170x956", "--ui-snapshot"])
            .arg(&out)
            .env("SSD_DATA_DIR", data.path())
            .output()
            .unwrap();
        let stderr = String::from_utf8_lossy(&run.stderr);
        assert_eq!(run.status.code(), Some(0), "{name}: stderr: {stderr}");
        let (width, height, rgba) = ssd_hd::png::read_rgba(&std::fs::read(&out).unwrap()).unwrap();
        let expected = decode(reference);
        let actual = signature(width, height, &rgba);
        let differences = differences(&actual, &expected);
        if !differences.is_empty() {
            problems.push(format!(
                "{name}: {}\n  its signature is {}",
                differences.join("; "),
                encode(&actual)
            ));
        }
        problems.extend(check_rendered(name, width, height, &rgba, &expected));
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

/// A stand-in for a frame: a dark ground with a gradient, blocks of 4 pixels in a few colours,
/// glyph-like cells of 32 pixels in two colours, and a window with a cream border.
fn synthetic_picture(width: usize, height: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(width * height * 4);
    for y in 0..height {
        for x in 0..width {
            let (bx, by) = (x / 4, y / 4);
            let hash = (bx * 73 + by * 151 + (bx * by) % 17) % 97;
            let mut pixel = [20 + (x / 8 % 16) as u8, 30, 60, 255];
            if (100..500).contains(&x) && (200..400).contains(&y) {
                pixel = if (104..496).contains(&x) && (204..396).contains(&y) {
                    [40, 40, 90, 255]
                } else {
                    [230, 220, 180, 255]
                };
            } else if (x / 32 + y / 32) % 3 == 0 && hash % 2 == 0 {
                let glyph = ((x / 4) % 8 + (y / 4) % 8 * 3 + hash) % 5 < 2;
                pixel = if glyph { [240, 230, 200, 255] } else { [10, 10, 40, 255] };
            }
            out.extend_from_slice(&pixel);
        }
    }
    out
}

/// The synthetic picture at the size of the snapshots, and its signature.
fn synthetic_frame() -> (Vec<u8>, Signature) {
    let (width, height) = (1170, 956);
    let picture = synthetic_picture(width, height);
    let expected = signature(width as u32, height as u32, &picture);
    (picture, expected)
}

#[test]
fn gpu_rounding_on_one_percent_of_the_pixels_matches() {
    let (picture, expected) = synthetic_frame();
    let rounded = rounded_differently(&picture, 7, 10);
    let actual = signature(expected.width, expected.height, &rounded);
    assert!(differences(&actual, &expected).is_empty(), "{:?}", differences(&actual, &expected));
}

#[test]
fn a_picture_shifted_by_one_pixel_fails() {
    let (picture, expected) = synthetic_frame();
    let shifted = shifted_right(&picture, expected.width as usize);
    let actual = signature(expected.width, expected.height, &shifted);
    assert!(differs(&actual, &expected));
}

#[test]
fn a_blanked_text_cell_fails() {
    let (picture, expected) = synthetic_frame();
    let (w, h) = (expected.width as usize, expected.height as usize);
    let at = busiest_square(&picture, w, h);
    let blank = blanked(&picture, w, at, 32);
    let actual = signature(expected.width, expected.height, &blank);
    assert!(!differences(&actual, &expected).is_empty(), "the cell at {at:?}");
}

#[test]
fn a_recoloured_window_fails() {
    let (picture, expected) = synthetic_frame();
    let (w, h) = (expected.width as usize, expected.height as usize);
    let recolour = recoloured(&picture, w, (w / 4, h / 4), (w / 2, h / 2));
    let actual = signature(expected.width, expected.height, &recolour);
    assert!(differs(&actual, &expected));
}

#[test]
fn a_filtered_picture_fails() {
    let (picture, expected) = synthetic_frame();
    let (w, h) = (expected.width as usize, expected.height as usize);
    let filtered = blurred(&picture, w, h);
    let actual = signature(expected.width, expected.height, &filtered);
    assert!(differs(&actual, &expected));
}

#[test]
fn a_picture_of_another_size_fails() {
    let (picture, expected) = synthetic_frame();
    let smaller = signature(expected.width - 1, expected.height, &picture[..4 * 1169 * 956]);
    assert!(differs(&smaller, &expected));
}
