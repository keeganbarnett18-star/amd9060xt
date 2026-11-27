//! A stand-in for the indirect light the title traces rays for.
//!
//! With the ray-tracing structure builder off (`crate::pipeline::builder_whole`)
//! the title's rays find nothing. Its program that resolves the diffuse light
//! of its screen probes (`CS_ScreenProbes_TemporalResolve_*`) then leaves the
//! sky's light outdoors, nothing indoors (walls are black where no lamp or
//! sun reaches) and, in blocks of the picture's tiles, what the probes'
//! memory held. Right after that program the light is raised to an estimate
//! and bounded by a multiple of it.
//!
//! That a stand-in can be put there, and in which of the program's images,
//! is beloooo's find (VIctory-Belobug-AMD, 06031237, 4f45ab88, cd9f78ce,
//! GPL-3.0-or-later), which puts a constant there, shaded in creases from the
//! scene's depth. The estimate here is worked out from the frame instead, so
//! it follows the hour, the room and its lamps by itself: the light the
//! visible surfaces send out (the title keeps it for its own screen traces:
//! `CS_CacheScreenSpaceRadiance`, the frame before) is averaged in 64x36
//! cells, and every point gets the mean of the cells weighted by the share of
//! its surroundings they cover, as a directional value (first-order harmonics
//! at 160x90 points, taken between them by depth).
//!
//! What it assumes: what is not on screen sends out what is (with a slow
//! mean of the last seconds, so that turning into a dark corner is not black
//! at once). No occlusion but the creases'.
//!
//! `VICTORY_AMBIENT`:
//! - unset or `0`/`off`: the title's own output.
//! - a number: that share of the estimate (1: all of it).
//! - `constant:<level>`: the constant, as beloooo's fork has it.
//!
//! `VICTORY_AMBIENT_SHADE=<strength>,<radius in metres>` sets the creases.
//! `VICTORY_AMBIENT_MOST=<level>` is the estimate's ceiling, in the
//! constant's units; `VICTORY_AMBIENT_LEAST=<level>` its floor.
//!
//! The air of a room is lit by the same probes: without rays they hold the
//! sky's light indoors too, a brown veil at sunset. `VICTORY_AMBIENT_AIR`
//! bounds what the air scatters by a multiple of the light around.

use std::sync::{Mutex, OnceLock};

use windows::Win32::Graphics::Direct3D12::*;
use windows::core::s;

use crate::host::{HostList, host};

/// Programs of the title the stand-in follows.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Role {
    /// Resolves the diffuse light.
    Resolve,
    /// Keeps the light visible surfaces send out.
    Radiance,
    /// Adds the lamps' light to what the air scatters (the last program
    /// that writes that volume before it is summed up along the view).
    Air,
    /// Writes the title's reflections for the last time in a frame; the
    /// lighting reads them next.
    Reflect,
}

/// The program's part in this, by its name in the container.
pub(crate) fn role(container: &[u8]) -> Option<Role> {
    let has = |name: &[u8]| container.windows(name.len()).any(|w| w == name);
    if has(b"CS_ScreenProbes_TemporalResolve") {
        Some(Role::Resolve)
    } else if has(b"CS_CacheScreenSpaceRadiance") {
        Some(Role::Radiance)
    } else if has(b"CS_AccumulateLocalLightInscatteringVolume") {
        Some(Role::Air)
    } else if has(b"CS_UpdateHistory") {
        Some(Role::Reflect)
    } else {
        None
    }
}

/// Where [`Role::Reflect`]'s program writes the reflections (R9G9B9E5 as
/// 32-bit integers, as the light): root parameter and place in its table
/// (beloooo's 44dcd437).
pub(crate) const REFLECTIONS: (u8, u32) = (1, 40);

/// The most and the least a reflection holds, as parts of the stand-in's
/// light around: `VICTORY_AMBIENT_REFLECT` (0.5) and
/// `VICTORY_AMBIENT_REFLECT_FLOOR` (0.15); 0 leaves that side alone.
///
/// Without rays the reflections are what the title's misses give: the sky,
/// unoccluded, on glossy surfaces (indoors a white glaze five to ten times
/// brighter than the room: beloooo's 44dcd437, which holds them to half the
/// light), and nothing elsewhere, so that paint and metal, whose light is
/// mostly their reflection, are black (halfwave50's eea5e3b0, a floor of
/// 0.15 in its launcher; session play52, the front half of a car beside the
/// player black, its back half lit).
fn reflect_hold() -> [f32; 2] {
    static SET: OnceLock<[f32; 2]> = OnceLock::new();
    *SET.get_or_init(|| {
        let read = |name: &str, default: f32| {
            std::env::var(name)
                .ok()
                .and_then(|v| v.trim().parse().ok())
                .filter(|v: &f32| v.is_finite() && *v >= 0.0)
                .unwrap_or(default)
        };
        [read("VICTORY_AMBIENT_REFLECT", 0.5), read("VICTORY_AMBIENT_REFLECT_FLOOR", 0.15)]
    })
}

/// Where the resolve writes the diffuse light (R9G9B9E5 as 32-bit integers)
/// and reads the scene's depth, and where the other writes the light of
/// visible surfaces: root parameter and place in its table, in dwords.
pub(crate) const LIGHT: (u8, u32) = (1, 176);
pub(crate) const DEPTH: (u8, u32) = (0, 464);
pub(crate) const RADIANCE: (u8, u32) = (1, 16);
/// Where the light the air scatters is.
pub(crate) const AIR: (u8, u32) = (1, 32);

#[derive(Clone, Copy, PartialEq, Debug)]
enum Set {
    Off,
    Estimate(f32),
    Constant(f32),
}

fn parse(set: &str) -> Set {
    let set = set.trim();
    let number = |t: &str| t.trim().parse::<f32>().ok().filter(|v| v.is_finite() && *v > 0.0);
    match set.strip_prefix("constant:") {
        Some(level) => number(level).map_or(Set::Off, Set::Constant),
        None => number(set).map_or(Set::Off, Set::Estimate),
    }
}

fn set() -> Set {
    static SET: OnceLock<Set> = OnceLock::new();
    *SET.get_or_init(|| match parse(&std::env::var("VICTORY_AMBIENT").unwrap_or_default()) {
        // With the band asked for and no share, the band's level.
        Set::Off => rt_band().map_or(Set::Off, |b| Set::Estimate(b.level)),
        set => set,
    })
}

/// With ray tracing (`VICTORY_AMBIENT_RT=<least>,<most>[,<level>]`; unset:
/// off, the title's own light whenever the builder runs): the diffuse light
/// the title's resolve leaves is kept between `least` and `most` times the
/// stand-in's light (level: the share of the estimate when `VICTORY_AMBIENT`
/// is not set, 0.8). With the builder running the title's traced diffuse
/// light flickers and goes black indoors (the diner of the opening: walls and
/// ceiling black under lit lamps) while the structures its rays walk are
/// partly wrong: its detail stays inside the band, its swings and black rooms
/// do not. On an RX 9060 XT a top of 2 took away white blotches the traced
/// light left.
///
/// From avatarmanz/VIctory-7900XTX 77137b98 (GPL-3.0-or-later), onto our
/// stand-in: with `constant:<level>` the constant is what the band is of
/// (without rays it is the light, as before); `VICTORY_AMBIENT_MOST`/
/// `LEAST` still bound the estimate; and with rays the air's bound and the
/// reflections' floor and cap stay off (`after`): they stand in for what
/// rays that find nothing leave, and would dim traced reflections.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) struct Band {
    least: f32,
    most: f32,
    level: f32,
}

pub(crate) fn rt_band() -> Option<Band> {
    static SET: OnceLock<Option<Band>> = OnceLock::new();
    *SET.get_or_init(|| {
        let band = parse_band(&std::env::var("VICTORY_AMBIENT_RT").ok()?)?;
        tracing::info!(least = band.least, most = band.most, level = band.level, "the stand-in keeps the traced indirect light in a band (VICTORY_AMBIENT_RT)");
        Some(band)
    })
}

fn parse_band(set: &str) -> Option<Band> {
    let mut parts = set.split(',').map(|p| p.trim().parse::<f32>().ok().filter(|v| v.is_finite() && *v >= 0.0));
    let least = parts.next().flatten()?;
    let most = parts.next().flatten().unwrap_or(4.0).max(least).max(1e-3);
    let level = parts.next().flatten().filter(|v| *v > 0.0).unwrap_or(0.8);
    Some(Band { least, most, level })
}

/// Whether the title's rays have structures to find: the builder has run
/// whole (`crate::pipeline::builder_whole`, which makes nothing).
fn rays() -> bool {
    crate::pipeline::builder_whole()
}

/// The title's leftover light is kept between these multiples of the
/// stand-in's: with rays, the band's; without, at least it and at most four
/// times it (as before).
fn bounds() -> (f32, f32) {
    match rt_band() {
        Some(b) if rays() => (b.least, b.most),
        _ => (1.0, 4.0),
    }
}

/// Whether the stand-in is on: set, and the title's rays find nothing (the
/// structure builder has not run whole: off, or its pipelines not all made
/// yet). Asked at the dispatch of the probes' resolve: makes nothing there
/// (`builder_whole`, not `builder_runs`), so a recording thread does not
/// wait for the builder's pipelines (notquiteog/victory-nvidia 3f8d7ecb). Or
/// the band is asked for with rays (`VICTORY_AMBIENT_RT`).
pub(crate) fn on() -> bool {
    set() != Set::Off && (rt_band().is_some() || !rays())
}

/// The most the air scatters, as a multiple of the light around
/// (`VICTORY_AMBIENT_AIR=<multiple>`; unset or 0: what the title has).
fn air() -> f32 {
    static SET: OnceLock<f32> = OnceLock::new();
    *SET.get_or_init(|| {
        let set = std::env::var("VICTORY_AMBIENT_AIR").unwrap_or_default();
        set.trim().parse().ok().filter(|v: &f32| v.is_finite() && *v > 0.0).unwrap_or(0.0)
    })
}

/// The brightest the estimate makes the light, in the constant's units
/// (`VICTORY_AMBIENT_MOST=<level>`; unset or 0: no ceiling). With the
/// estimate (`VICTORY_AMBIENT=1`) and a ceiling of the constant's 0.8, the
/// light follows the hour, the room and its lamps below that level and the
/// air is bounded by it (`VICTORY_AMBIENT_AIR`, which the constant leaves
/// alone), while a bright doorway filling the view no longer washes the room
/// out (why the launchers took the constant, 22d4a742).
fn ceiling() -> f32 {
    static SET: OnceLock<f32> = OnceLock::new();
    *SET.get_or_init(|| {
        let set = std::env::var("VICTORY_AMBIENT_MOST").unwrap_or_default();
        set.trim().parse().ok().filter(|v: &f32| v.is_finite() && *v > 0.0).unwrap_or(0.0)
    })
}

/// The darkest the estimate makes the light, in the constant's units
/// (`VICTORY_AMBIENT_LEAST=<level>`; unset or 0: no floor). The estimate
/// alone left shade outdoors in the day all but black (session play51, the
/// street corner at 1:39): the cells it averages hold less light than the
/// constant's units suppose.
fn least() -> f32 {
    static SET: OnceLock<f32> = OnceLock::new();
    *SET.get_or_init(|| {
        let set = std::env::var("VICTORY_AMBIENT_LEAST").unwrap_or_default();
        set.trim().parse().ok().filter(|v: &f32| v.is_finite() && *v > 0.0).unwrap_or(0.0)
    })
}

/// Strength and radius of the creases' shade.
fn shade() -> [f32; 2] {
    static SET: OnceLock<[f32; 2]> = OnceLock::new();
    *SET.get_or_init(|| {
        let mut shade = [1.0, 0.6];
        let set = std::env::var("VICTORY_AMBIENT_SHADE").unwrap_or_default();
        for (to, v) in shade.iter_mut().zip(set.split(',')) {
            if let Ok(v) = v.trim().parse() {
                *to = v;
            }
        }
        shade
    })
}

const SOURCE: &str = r#"
#define RS "RootConstants(num32BitConstants=17, b0), DescriptorTable(SRV(t0)), DescriptorTable(UAV(u0)), UAV(u1), UAV(u2)"
#define CX 64
#define CY 36
#define CELLS (CX * CY)
#define PX 160
#define PY 90
cbuffer Set : register(b0) {
    float level; float constant; float strength; float radius;
    float near; float tan_y; uint width; uint height;
    float follow; float most; float air; float ceiling;
    float floor_; float reflect_most; float reflect_least;
    // The band with rays (`VICTORY_AMBIENT_RT`): the least part of the
    // stand-in's light kept, and whether rays find structures (1) or not.
    float lowest; uint rays;
};
Texture2D<float> depth : register(t0);
// Cells: the light they send out (then, at CELLS, the slow mean), from
// CELLS + 1 on where they are.
RWStructuredBuffer<float4> cells : register(u1);
// Three a point: red, green and blue, each a mean and a direction.
RWStructuredBuffer<float4> points : register(u2);
static const float3 LUMA = float3(0.2126, 0.7152, 0.0722);

// The point seen at a texel, in metres from the eye (x right, y up, z away).
float3 seen(int2 p, float2 size) {
    float d = depth.Load(int3(clamp(p, int2(0, 0), int2(size) - 1), 0));
    float z = min(near / max(d, 1e-7), 1000.0);
    float2 ndc = (float2(p) + 0.5) / size * 2.0 - 1.0;
    return float3(ndc.x * tan_y * size.x / size.y * z, -ndc.y * tan_y * z, z);
}

#ifdef KEEP
RWTexture2D<float3> radiance : register(u0);
[RootSignature(RS)]
[numthreads(8, 4, 1)]
void keep(uint3 at : SV_DispatchThreadID) {
    uint w, h;
    radiance.GetDimensions(w, h);
    float3 sum = 0;
    for (uint y = 0; y < 5; y++) for (uint x = 0; x < 5; x++) {
        uint2 p = uint2((float2(at.xy) + (float2(x, y) + 0.5) / 5.0) * float2(w, h) / float2(CX, CY));
        float3 c = max(radiance[p], 0.0);
        // NaN or infinity: every exponent bit set (no `isnan`/`isinf`, which
        // Wine's compiler lacks).
        c = any((asuint(c) & 0x7f800000u) == 0x7f800000u) ? 0.0 : c;
        // Lamps and glints count, but not for everything.
        sum += c / max(1.0, dot(c, LUMA) / 20.0);
    }
    cells[at.y * CX + at.x] = float4(sum / 25.0, 1.0);
}
[RootSignature(RS)]
[numthreads(1, 1, 1)]
void mean() {
    float3 sum = 0;
    for (uint i = 0; i < CELLS; i++) sum += cells[i].rgb;
    float4 was = cells[CELLS];
    float3 now = sum / CELLS;
    cells[CELLS] = float4(was.a > 0.0 ? lerp(was.rgb, now, follow) : now, 1.0);
}
#endif

#ifdef PLACE
[RootSignature(RS)]
[numthreads(8, 4, 1)]
void place(uint3 at : SV_DispatchThreadID) {
    float2 size = float2(width, height);
    float3 p = seen(int2((float2(at.xy) + 0.5) * size / float2(CX, CY)), size);
    cells[CELLS + 1 + at.y * CX + at.x] = float4(p, p.z * p.z);
}
#endif

#ifdef GATHER
[RootSignature(RS)]
[numthreads(8, 2, 1)]
void gather(uint3 at : SV_DispatchThreadID) {
    float2 size = float2(width, height);
    float3 p = seen(int2((float2(at.xy) + 0.5) * size / float2(PX, PY)), size);
    float r2 = 0.25 + 0.0025 * p.z * p.z;
    float whole = 0;
    float3 toward = 0, light = 0, red = 0, green = 0, blue = 0;
    for (uint i = 0; i < CELLS; i++) {
        float4 c = cells[CELLS + 1 + i];
        float3 L = cells[i].rgb;
        float3 d = c.xyz - p;
        float dd = dot(d, d);
        float w = c.w / (dd + r2);
        float3 dir = d / sqrt(dd + 0.25 * r2);
        whole += w;
        toward += w * dir;
        light += w * L;
        red += w * L.r * dir;
        green += w * L.g * dir;
        blue += w * L.b * dir;
    }
    whole = max(whole, 1e-6);
    light /= whole;
    toward /= whole;
    uint at0 = (at.y * PX + at.x) * 3;
    points[at0] = float4(red / whole - light.r * toward, light.r);
    points[at0 + 1] = float4(green / whole - light.g * toward, light.g);
    points[at0 + 2] = float4(blue / whole - light.b * toward, light.b);
}
#endif

#ifdef AIR
RWTexture3D<float3> scattered : register(u0);
// No more than a multiple of the light around: of the room as the last
// seconds showed it, and of the surface seen in this direction (towards a
// window the air outside may be as bright as the sky is).
[RootSignature(RS)]
[numthreads(8, 8, 2)]
void clear_air(uint3 at : SV_DispatchThreadID) {
    uint w, h, d;
    scattered.GetDimensions(w, h, d);
    if (at.x >= w || at.y >= h || at.z >= d) return;
    uint2 q = min(uint2((float2(at.xy) + 0.5) / float2(w, h) * float2(PX, PY)), uint2(PX - 1, PY - 1));
    uint at0 = (q.y * PX + q.x) * 3;
    float3 seen_there = float3(points[at0].w, points[at0 + 1].w, points[at0 + 2].w);
    float far = float(at.z) / float(d);
    float cap = air * lerp(dot(cells[CELLS].rgb, LUMA), dot(seen_there, LUMA), far);
    float3 c = scattered[at];
    float has = dot(c, LUMA);
    if (has > cap) scattered[at] = c * (cap / has);
}
#endif

#if defined(RAISE) || defined(HOLD)
RWTexture2D<uint> light : register(u0);
float3 unpack(uint v) {
    return float3(v & 511, (v >> 9) & 511, (v >> 18) & 511) * exp2(float(v >> 27) - 24.0);
}
uint pack(float3 c) {
    c = clamp(c, 0.0, 65408.0);
    float top = max(c.r, max(c.g, c.b));
    int e = clamp(int(floor(log2(max(top, 1e-9)))) + 16, 0, 31);
    float scale = exp2(float(24 - e));
    if (top * scale >= 511.5) { e = min(e + 1, 31); scale = exp2(float(24 - e)); }
    uint3 m = min(uint3(c * scale + 0.5), 511u);
    return m.r | (m.g << 9) | (m.b << 18) | (uint(e) << 27);
}
#endif

#ifdef HOLD
// The reflections the lighting reads next, held between a floor and a cap,
// both parts of the light around: the constant's, or the estimate's slow
// mean. What is under the floor is lifted to it in the light's colour.
[RootSignature(RS)]
[numthreads(8, 8, 1)]
void hold(uint3 at : SV_DispatchThreadID) {
    if (at.x >= width || at.y >= height) return;
    float3 around = constant > 0.0 ? float3(1.0, 0.98, 0.94) * constant : cells[CELLS].rgb * level;
    float a = dot(around, LUMA);
    if (a <= 0.0) return;
    float3 c = unpack(light[at.xy]);
    float has = dot(c, LUMA);
    if (reflect_most > 0.0 && has > a * reflect_most) {
        light[at.xy] = pack(c * (a * reflect_most / has));
    } else if (reflect_least > 0.0 && has < a * reflect_least) {
        light[at.xy] = pack(c + around * ((a * reflect_least - has) / a));
    }
}
#endif

#ifdef RAISE
[RootSignature(RS)]
[numthreads(8, 8, 1)]
void raise(uint3 at : SV_DispatchThreadID) {
    float2 size = float2(width, height);
    if (at.x >= width || at.y >= height) return;
    int2 p = int2(at.xy);
    float3 c = seen(p, size);
    float3 l = seen(p + int2(-1, 0), size), r = seen(p + int2(1, 0), size);
    float3 u = seen(p + int2(0, -1), size), d = seen(p + int2(0, 1), size);
    float3 dx = abs(l.z - c.z) < abs(r.z - c.z) ? c - l : r - c;
    float3 dy = abs(u.z - c.z) < abs(d.z - c.z) ? c - u : d - c;
    float3 n = normalize(cross(dy, dx) + float3(0, 0, -1e-9));
    if (dot(n, c) > 0.0) n = -n;
    // How open it is (as in beloooo's cd9f78ce).
    float open = 1.0;
    float px = min(radius * size.y / (2.0 * tan_y * c.z), 128.0);
    float noise = frac(52.9829189 * frac(dot(float2(p), float2(0.06711056, 0.00583715))));
    if (strength > 0.0 && px >= 1.5 && c.z < 400.0) {
        float sum = 0.0;
        const int N = 12;
        for (int i = 0; i < N; i++) {
            float a = i * 2.399963 + noise * 6.2831853;
            float2 o = float2(cos(a), sin(a)) * (sqrt((i + 0.5) / N) * px);
            float3 v = seen(p + int2(round(o)), size) - c;
            float vv = dot(v, v);
            float over = dot(v, n) - 0.004 * c.z - 0.005;
            sum += max(over, 0.0) / sqrt(vv + 1e-6) * saturate(1.0 - vv / (radius * radius));
        }
        open = saturate(1.0 - strength * 2.0 * sum / N);
    }
    float3 found;
    if (constant > 0.0) {
        found = float3(1.0, 0.98, 0.94) * constant * open;
        // Without rays the constant is the light; with them it is what the
        // band is of (`VICTORY_AMBIENT_RT`).
        if (rays == 0u) {
            light[p] = pack(found);
            return;
        }
    } else {
        // Between the four points around, those at its own depth first; which
        // four is dithered a little.
        float2 g = (float2(p) + 0.5) / size * float2(PX, PY) - 0.5 + (noise - 0.5) * 0.5;
        int2 g0 = int2(floor(g));
        float2 f = g - float2(g0);
        float4 sr = 0, sg = 0, sb = 0;
        float sum = 0;
        for (int k = 0; k < 4; k++) {
            int2 q = clamp(g0 + int2(k & 1, k >> 1), int2(0, 0), int2(PX, PY) - 1);
            float z = seen(int2((float2(q) + 0.5) * size / float2(PX, PY)), size).z;
            float2 b = float2(k & 1 ? f.x : 1 - f.x, k >> 1 ? f.y : 1 - f.y);
            float w = (b.x * b.y + 1e-3) / (0.02 + abs(z - c.z) / c.z);
            uint at0 = (q.y * PX + q.x) * 3;
            sr += w * points[at0];
            sg += w * points[at0 + 1];
            sb += w * points[at0 + 2];
            sum += w;
        }
        float3 all = float3(sr.w, sg.w, sb.w) / sum;
        found = all + 1.2 * float3(dot(sr.xyz, n), dot(sg.xyz, n), dot(sb.xyz, n)) / sum;
        found = clamp(found, 0.2 * all, 2.5 * all);
        found = lerp(found, cells[CELLS].rgb, 0.2) * level * open;
        // No brighter than the ceiling (`VICTORY_AMBIENT_MOST`): a bright
        // doorway filling the view raises the cells, not the room past it.
        float lum = dot(found, LUMA);
        if (ceiling > 0.0 && lum > ceiling * open) found *= ceiling * open / lum;
        // No darker than the floor (`VICTORY_AMBIENT_LEAST`): shade outdoors in
        // the day was all but black with the estimate alone (session play51);
        // what is missing is made up in the constant's tint.
        lum = dot(found, LUMA);
        if (floor_ > 0.0 && lum < floor_ * open) found += float3(1.0, 0.98, 0.94) * (floor_ * open - lum);
    }
    float3 was = unpack(light[p]);
    // What the program left counts as far as it can be true: the sky's light
    // outdoors, not the blocks of other memory. With rays, between the
    // band's least and most parts of it.
    float top = max(dot(found, LUMA) * most, 1e-6);
    float has = dot(was, LUMA);
    was *= min(1.0, top / max(has, 1e-6));
    light[p] = pack(max(was, found * lowest));
}
#endif
"#;

/// Bytes of the cells' buffer and of the points'.
const CELLS: u64 = 64 * 36;
const CELL_BYTES: u64 = (2 * CELLS + 1) * 16;
const POINT_BYTES: u64 = 160 * 90 * 48;

struct Pass {
    root: ID3D12RootSignature,
    keep: ID3D12PipelineState,
    mean: ID3D12PipelineState,
    place: ID3D12PipelineState,
    gather: ID3D12PipelineState,
    raise: ID3D12PipelineState,
    air: ID3D12PipelineState,
    hold: ID3D12PipelineState,
    cells: ID3D12Resource,
    points: ID3D12Resource,
}
// SAFETY: D3D12 objects are free-threaded.
unsafe impl Send for Pass {}
// SAFETY: as above.
unsafe impl Sync for Pass {}

fn make(device: &ID3D12Device, buffer: impl Fn(u64) -> Option<ID3D12Resource>) -> windows::core::Result<Pass> {
    let mut root: Option<ID3D12RootSignature> = None;
    let mut pipeline = |part: &str, entry| -> windows::core::Result<ID3D12PipelineState> {
        let code = crate::host::compile(&format!("#define {part}\n{SOURCE}"), entry, s!("cs_5_1"))?;
        // SAFETY: the root signature is inside the shader; plain creations.
        unsafe {
            if root.is_none() {
                // As the shader's `RS`, made here: Wine's compiler embeds none.
                use crate::rootsig::Param;
                root = Some(crate::rootsig::make(
                    device,
                    &[
                        Param::Constants { register: 0, count: 17 },
                        Param::SrvTable { register: 0, count: 1, visibility: D3D12_SHADER_VISIBILITY_ALL },
                        Param::UavTable { register: 0, count: 1 },
                        Param::Uav(1),
                        Param::Uav(2),
                    ],
                    "indirect light stand-in",
                )?);
            }
            let desc = D3D12_COMPUTE_PIPELINE_STATE_DESC {
                pRootSignature: std::mem::transmute_copy(&root),
                CS: D3D12_SHADER_BYTECODE { pShaderBytecode: code.as_ptr().cast(), BytecodeLength: code.len() },
                ..Default::default()
            };
            device.CreateComputePipelineState(&desc)
        }
    };
    let keep = pipeline("KEEP", s!("keep"))?;
    let mean = pipeline("KEEP", s!("mean"))?;
    let place = pipeline("PLACE", s!("place"))?;
    let gather = pipeline("GATHER", s!("gather"))?;
    let raise = pipeline("RAISE", s!("raise"))?;
    let air = pipeline("AIR", s!("clear_air"))?;
    let hold = pipeline("HOLD", s!("hold"))?;
    let made = |bytes| buffer(bytes).ok_or_else(windows::core::Error::empty);
    Ok(Pass {
        root: root.ok_or_else(windows::core::Error::empty)?,
        keep,
        mean,
        place,
        gather,
        raise,
        air,
        hold,
        cells: made(CELL_BYTES)?,
        points: made(POINT_BYTES)?,
    })
}

fn pass() -> Option<&'static Pass> {
    static PASS: OnceLock<Option<Pass>> = OnceLock::new();
    PASS.get_or_init(|| {
        let host = host()?;
        make(&host.device, |bytes| host.uav_buffer(bytes))
            .inspect_err(|e| tracing::warn!(error = %e, "no stand-in for the indirect light"))
            .ok()
    })
    .as_ref()
}

fn uav_barrier(list: &ID3D12GraphicsCommandList) {
    let barrier = D3D12_RESOURCE_BARRIER { Type: D3D12_RESOURCE_BARRIER_TYPE_UAV, ..Default::default() };
    // SAFETY: an open list; the barrier names no resource.
    unsafe { list.ResourceBarrier(&[barrier]) };
}

/// Whether the light of a frame has been kept: the estimate has something
/// to begin with.
static KEPT: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// What the passes read and write, as views in the heap the list has set.
pub(crate) struct Views {
    /// The image the title's program just wrote.
    pub(crate) image: D3D12_GPU_DESCRIPTOR_HANDLE,
    /// The scene's depth, for [`Role::Resolve`].
    pub(crate) depth: D3D12_GPU_DESCRIPTOR_HANDLE,
    /// The size of the image, and how many slices it has.
    pub(crate) size: [u32; 2],
    pub(crate) slices: u32,
}

/// After the title's program of that role, in `list` with the program's view
/// heap set. Replaces the list's compute root signature and pipeline; false
/// if it recorded nothing.
pub(crate) fn after(list: &HostList, role: Role, views: &Views) -> bool {
    use std::sync::atomic::Ordering;
    let Some(pass) = pass() else {
        return false;
    };
    let (level, constant) = match set() {
        Set::Off => return false,
        Set::Estimate(level) => (level, 0.0f32),
        Set::Constant(level) => (0.0, level),
    };
    // The air is bounded by what the estimate gathers, so it waits for a
    // kept frame with the constant too.
    if role != Role::Radiance && (constant == 0.0 || role == Role::Air) && !KEPT.load(Ordering::Relaxed) {
        return false;
    }
    // With the constant the air is bounded as well (`VICTORY_AMBIENT_AIR`):
    // without it the haze kept the probes' sky light and other memory and
    // changed from frame to frame (session play50, the camp).
    if role == Role::Air && air() == 0.0 {
        return false;
    }
    if role == Role::Reflect && reflect_hold() == [0.0, 0.0] {
        return false;
    }
    // With rays (the band, `VICTORY_AMBIENT_RT`) the air and the reflections
    // are the title's: their bounds stand in for rays that find nothing.
    let rays = rays();
    if rays && matches!(role, Role::Air | Role::Reflect) {
        return false;
    }
    static SAID: Mutex<u32> = Mutex::new(0);
    {
        let mut said = SAID.lock().unwrap_or_else(|e| e.into_inner());
        if *said < 4 {
            *said += 1;
            tracing::info!(?role, size = ?views.size, level, constant, "a stand-in for the indirect light");
        }
    }
    list.keep(&pass.cells);
    list.keep(&pass.points);
    let [strength, radius] = shade();
    let floats = [level, constant, strength, radius, 0.1, 0.45];
    let mut words = [0u32; 17];
    for (to, v) in words.iter_mut().zip(floats) {
        *to = v.to_bits();
    }
    words[6] = views.size[0];
    words[7] = views.size[1];
    words[8] = 0.04f32.to_bits();
    let (lowest, most) = bounds();
    words[9] = most.to_bits();
    words[10] = air().to_bits();
    words[11] = ceiling().to_bits();
    words[12] = least().to_bits();
    let [reflect_most, reflect_least] = reflect_hold();
    words[13] = reflect_most.to_bits();
    words[14] = reflect_least.to_bits();
    words[15] = lowest.to_bits();
    words[16] = u32::from(rays && rt_band().is_some());
    let l = &list.list;
    // SAFETY: an open list with the heap of the views set; the title's
    // program has just used both views, so their textures are in the states
    // these views need; buffers the list keeps.
    unsafe {
        uav_barrier(l);
        l.SetComputeRootSignature(&pass.root);
        l.SetComputeRoot32BitConstants(0, 17, words.as_ptr().cast(), 0);
        l.SetComputeRootDescriptorTable(1, views.depth);
        l.SetComputeRootDescriptorTable(2, views.image);
        l.SetComputeRootUnorderedAccessView(3, pass.cells.GetGPUVirtualAddress());
        l.SetComputeRootUnorderedAccessView(4, pass.points.GetGPUVirtualAddress());
        let run = |pso: &ID3D12PipelineState, x, y, z| {
            l.SetPipelineState(pso);
            l.Dispatch(x, y, z);
            uav_barrier(l);
        };
        match role {
            Role::Radiance => {
                run(&pass.keep, 8, 9, 1);
                run(&pass.mean, 1, 1, 1);
                KEPT.store(true, Ordering::Relaxed);
            }
            Role::Air => run(&pass.air, views.size[0].div_ceil(8), views.size[1].div_ceil(8), views.slices.div_ceil(2)),
            Role::Reflect => run(&pass.hold, views.size[0].div_ceil(8), views.size[1].div_ceil(8), 1),
            Role::Resolve => {
                // (The air's bound reads the points with the constant too.)
                if constant == 0.0 || air() > 0.0 {
                    run(&pass.place, 8, 9, 1);
                    run(&pass.gather, 20, 45, 1);
                }
                run(&pass.raise, views.size[0].div_ceil(8), views.size[1].div_ceil(8), 1);
            }
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings() {
        assert_eq!(parse(""), Set::Off);
        assert_eq!(parse("off"), Set::Off);
        assert_eq!(parse("0"), Set::Off);
        assert_eq!(parse("0.8"), Set::Estimate(0.8));
        assert_eq!(parse("constant:0.3"), Set::Constant(0.3));
        assert_eq!(parse("constant:nan"), Set::Off);
    }

    #[test]
    fn the_band_with_rays() {
        assert_eq!(parse_band("0.5,1.5"), Some(Band { least: 0.5, most: 1.5, level: 0.8 }));
        assert_eq!(parse_band("1,1,0.6"), Some(Band { least: 1.0, most: 1.0, level: 0.6 }));
        // The top is never under the floor; a missing top is four times.
        assert_eq!(parse_band("2,1"), Some(Band { least: 2.0, most: 2.0, level: 0.8 }));
        assert_eq!(parse_band("0.7"), Some(Band { least: 0.7, most: 4.0, level: 0.8 }));
        assert_eq!(parse_band(""), None);
        assert_eq!(parse_band("x,1"), None);
    }

    #[test]
    fn roles_by_name() {
        assert_eq!(role(b"..CS_ScreenProbes_TemporalResolve_a911b49c_Wrapped.."), Some(Role::Resolve));
        assert_eq!(role(b"..CS_CacheScreenSpaceRadiance_eed2fc35.."), Some(Role::Radiance));
        assert_eq!(role(b"CS_AccumulateLocalLightInscatteringVolume_7e2f5eef"), Some(Role::Air));
        assert_eq!(role(b"..CS_UpdateHistory_3c1d.."), Some(Role::Reflect));
        assert_eq!(role(b"CS_TemporalResolve"), None);
    }

    /// The passes compile with the system's compiler and the host takes them.
    #[test]
    fn the_passes_build() {
        let Some(host) = host() else {
            return;
        };
        make(&host.device, |bytes| host.uav_buffer(bytes)).expect("the passes");
    }
}
