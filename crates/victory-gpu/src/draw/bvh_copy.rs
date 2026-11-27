//! The ray-tracing structure builder's copy before a refit, run by a pass
//! of ours (`VICTORY_BVH_COPY=native`; off by default), and a check of
//! that pass against the translated program in the title
//! (`VICTORY_BVH_COPY=check`).
//!
//! The builder's `CopyForBVHUpdate` (container hash [`HASH`]) copies each
//! structure before its refit, about 57 times a frame. It runs groups of
//! 32 threads; its root view `t0` is the source, `u0` the destination and
//! its root constants `c0`, `c1`. What it does, in our words:
//!
//! ```text
//! bytes = (src.dword[1] << 6) + 64                 // 32-bit arithmetic
//! for (at = thread * 16; at < bytes; at += c0 * 16)
//!     dst[at..at + 16] = src[at..at + 16]
//! if (thread == 0 && c1 != 0) dst gets bounds at bytes 16..40
//! ```
//!
//! `thread` is the group along x times 32 plus the lane, and `c0` is the
//! title's thread count (0x1000 or 0x4000; `c1` was 0 in every dispatch
//! logged). Translated, every 16 bytes copied cost five accesses to the
//! PC's memory besides the copy itself: the directory of title memory by
//! address for the load and for the store, and three notes (`crate::memory`,
//! `victory_shader` `lower.rs` `memory`). That made the program about
//! 28 ms of a frame's GPU time.
//!
//! **Ours** reads and writes the host copies the program would through
//! root views, with nothing else per 16 bytes, and loops exactly as the
//! program does: the same groups, the same `c0`, the same 32-bit sums, and
//! no cap on the loop (the translator stops a loop after 4,096 rounds;
//! the title does not). It also writes what the program's accesses would
//! have noted, once per dispatch rather than per access: the touch of
//! both copies and the mark of each 64 KiB page of the destination, which
//! decide which copies are brought up to date and copied back.
//!
//! **Same bytes.** A view is the copy [`crate::shadow::find`] gives its
//! address, as for the root views the title binds. A program reading title
//! memory by address finds a copy per 64 KiB page in the directory, so
//! ours reaches only as far as the directory gives the pages to that copy
//! at the offsets the copy holds them ([`crate::memory::run_in`]): no
//! further than the copy, no further than the other view (the ranges would
//! overlap), 256 MiB at most.
//!
//! **What falls back.** The translated program runs as before when:
//! - anything is unknown or unusual at recording ([`slots`], [`limit`]:
//!   a view not set, `c1` set, a step of 0, a view not on 16 bytes, the
//!   source and destination at one address, pages not in the directory);
//! - the length, read on the card, is more than ours may reach. Ours then
//!   copies nothing, sets its record's flag and counts it; the program's
//!   dispatch is predicated on that flag, so it runs only then.
//!
//! **The check** (`VICTORY_BVH_COPY=check`): ours copies into a buffer in
//! the card's memory (64 MiB, [`SCRATCH`]), the translated program runs as
//! before, and a compare counts the dwords that differ. The counts are
//! read back a frame later and logged, nothing else.
//!
//! From xeltax/vicky eb6a31a0 (GPL-3.0-or-later), ported by hand: our
//! directory's entries carry [`victory_shader::emit::MEMORY_READ_ONLY`] for
//! pages stores do not reach (ours then stops there for the destination),
//! our translated programs leave no touch or page marks ([`Notes`]), and the
//! dispatch's groups and partial threads come from our `dispatch_groups`.
//! Its tests pass on WARP and on an RX 9060 XT (`-- --ignored`).

use std::mem::ManuallyDrop;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

use victory_shader::{ResourceType, RootType, UsedResource};
use windows::Win32::Graphics::Direct3D12::*;
use windows::core::s;

use super::{Root, State};
use crate::host::HostList;
use crate::pipeline::host::{HostPipeline, Stage};

/// The builder's copy program, by its container's hash.
const HASH: &str = "c42a31951201f9a0";

/// Bytes ours reaches at most past each view.
const MOST: u64 = 256 << 20;

/// Rounds of a thread's loop at most: past that the program runs
/// translated (whose loop has a cap), so that a tiny `c0` cannot hold the
/// card for seconds.
const ROUNDS: u64 = 1 << 16;

/// Records, 16 bytes each, one per dispatch, used in turn: about 70
/// frames' worth.
const RECORDS: u32 = 4096;

/// The check's copy goes here: copies longer than this are not compared.
pub(super) const SCRATCH: u64 = 64 << 20;

/// Groups of 64 threads of the compare.
const COMPARE_GROUPS: u32 = 256;

/// The counters the passes add to, by byte offset in `counts`.
pub(super) mod count {
    /// Copies ours made.
    pub(crate) const DONE: usize = 0;
    /// Copies longer than ours could reach: left to the translated program.
    pub(crate) const OVER: usize = 4;
    /// Check: copies compared, their dwords, the dwords that differed and
    /// the copies that had any.
    pub(crate) const COMPARED: usize = 8;
    pub(crate) const DWORDS: usize = 12;
    pub(crate) const DIFFERING: usize = 16;
    pub(crate) const COPIES_DIFFERING: usize = 20;
    /// Check: copies not compared (ours did not copy).
    pub(crate) const NOT_COMPARED: usize = 24;
    /// Dwords of the counters.
    pub(crate) const ALL: usize = 7;
}

/// Our copy. Every view is a raw unordered access view (the host copies
/// are written as such by every translated program), from the byte the
/// title's view starts at. A record is 16 bytes: the flag (a 64-bit 0 or
/// 1: predication reads it), the length the program would copy, and the
/// number of compare groups that found a difference.
pub(super) const COPY: &str = r#"
#define RS "RootConstants(num32BitConstants=10, b0), UAV(u0), UAV(u1), UAV(u2), UAV(u3), UAV(u4)"
cbuffer Copy : register(b0) {
    uint threads;   // the title's c0: a thread's chunks are this many chunks apart
    uint limit;     // the most bytes the copy may cover
    uint record;    // this dispatch's record in `records`, in bytes
    uint lanes;     // threads dispatched along x
    uint marks;     // the mark of the destination's first 64 KiB page in `notes`, in bytes (0: none)
    uint skew;      // the destination's byte in that page
    uint src_touch; // the source copy's touch in `notes`, in bytes (0: none)
    uint src_page;  // what the touch is set to
    uint dst_touch;
    uint dst_page;
};
RWByteAddressBuffer src : register(u0);
RWByteAddressBuffer dst : register(u1);
RWByteAddressBuffer records : register(u2);
RWByteAddressBuffer counts : register(u3);
RWByteAddressBuffer notes : register(u4);
[RootSignature(RS)]
[numthreads(32, 1, 1)]
void main(uint3 id : SV_DispatchThreadID) {
    uint bytes = (src.Load(4) << 6) + 64u;
    bool over = bytes > limit;
    uint was;
    if (id.x == 0u && id.y == 0u && id.z == 0u) {
        records.Store4(record, uint4(over ? 1u : 0u, 0u, bytes, 0u));
        counts.InterlockedAdd(over ? 4u : 0u, 1u, was);
        if (!over && src_touch != 0u) notes.Store(src_touch, src_page);
        if (!over && dst_touch != 0u) notes.Store(dst_touch, dst_page);
    }
    if (over) return;
    uint step = threads << 4;
    [loop] [allow_uav_condition]
    for (uint at = id.x << 4; at < bytes;) {
        dst.Store4(at, src.Load4(at));
        // Past 4 GiB the title's sum would wrap and copy the same bytes again.
        uint next = at + step;
        if (next < at) break;
        at = next;
    }
    if (marks != 0u) {
        uint pages = (skew + bytes + 0xFFFFu) >> 16;
        [loop] [allow_uav_condition]
        for (uint page = id.x; page < pages; page += lanes) notes.Store(marks + page * 4u, 1u);
    }
}
"#;

/// The check's compare of our copy (`ours`) with the program's
/// (`theirs`), over the length our copy's record holds.
pub(super) const COMPARE: &str = r#"
#define RS "RootConstants(num32BitConstants=2, b0), UAV(u0), UAV(u1), UAV(u2), UAV(u3)"
cbuffer Compare : register(b0) {
    uint record; // the copy's record in `records`, in bytes
    uint lanes;  // threads dispatched
};
RWByteAddressBuffer ours : register(u0);
RWByteAddressBuffer theirs : register(u1);
RWByteAddressBuffer records : register(u2);
RWByteAddressBuffer counts : register(u3);
groupshared uint differing;
[RootSignature(RS)]
[numthreads(64, 1, 1)]
void main(uint3 id : SV_DispatchThreadID, uint gi : SV_GroupIndex) {
    if (gi == 0u) differing = 0u;
    GroupMemoryBarrierWithGroupSync();
    uint4 r = records.Load4(record);
    uint n = 0u;
    if (r.x == 0u) {
        [loop] [allow_uav_condition]
        for (uint at = id.x << 4; at < r.z; at += lanes << 4) {
            uint4 a = ours.Load4(at);
            uint4 b = theirs.Load4(at);
            n += (a.x != b.x ? 1u : 0u) + (a.y != b.y ? 1u : 0u) + (a.z != b.z ? 1u : 0u) + (a.w != b.w ? 1u : 0u);
        }
    }
    uint was;
    if (n != 0u) InterlockedAdd(differing, n, was);
    GroupMemoryBarrierWithGroupSync();
    if (gi == 0u && differing != 0u) {
        counts.InterlockedAdd(16u, differing, was);
        records.InterlockedAdd(record + 12u, 1u, was);
        if (was == 0u) counts.InterlockedAdd(20u, 1u, was);
    }
    if (id.x == 0u) {
        if (r.x == 0u) {
            counts.InterlockedAdd(8u, 1u, was);
            counts.InterlockedAdd(12u, r.z >> 2, was);
        } else {
            counts.InterlockedAdd(24u, 1u, was);
        }
    }
}
"#;

/// What `VICTORY_BVH_COPY` asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Mode {
    /// Ours copies; the program runs only where ours could not.
    Native,
    /// Ours copies aside, the program as before, and the two are compared.
    Check,
}

pub(super) fn parse_mode(value: &str) -> Option<Mode> {
    match value.trim() {
        "native" => Some(Mode::Native),
        "check" => Some(Mode::Check),
        _ => None,
    }
}

fn mode() -> Option<Mode> {
    static MODE: OnceLock<Option<Mode>> = OnceLock::new();
    *MODE.get_or_init(|| {
        let value = std::env::var("VICTORY_BVH_COPY").ok()?;
        let mode = parse_mode(&value);
        match mode {
            Some(mode) => tracing::info!(
                ?mode,
                "VICTORY_BVH_COPY: the structure builder's copy is run by a pass of ours"
            ),
            None if !value.trim().is_empty() => {
                tracing::warn!(
                    value,
                    "VICTORY_BVH_COPY is neither native nor check: the copy runs translated"
                )
            }
            None => {}
        }
        mode
    })
}

/// A root signature and pipeline of ours.
pub(super) struct Pass {
    pub(super) root: ID3D12RootSignature,
    pub(super) pso: ID3D12PipelineState,
}

/// Compiles `source` (with its root signature inside) for `device`, as the
/// host's other passes are ([`crate::host::compile`]).
pub(super) fn pass(device: &ID3D12Device, source: &str) -> windows::core::Result<Pass> {
    let code = crate::host::compile(source, s!("main"), s!("cs_5_1"))?;
    // SAFETY: the compiled bytes; the root signature is inside the shader.
    unsafe {
        let root: ID3D12RootSignature = device.CreateRootSignature(0, &code)?;
        let pso = device.CreateComputePipelineState(&D3D12_COMPUTE_PIPELINE_STATE_DESC {
            pRootSignature: std::mem::transmute_copy(&root),
            CS: D3D12_SHADER_BYTECODE {
                pShaderBytecode: code.as_ptr().cast(),
                BytecodeLength: code.len(),
            },
            ..Default::default()
        })?;
        Ok(Pass { root, pso })
    }
}

/// The passes and the buffers they share, made at the first copy.
struct Work {
    copy: Pass,
    compare: Pass,
    /// [`RECORDS`] records in the card's memory.
    records: ID3D12Resource,
    records_gpu: u64,
    /// The counters ([`count`]), in the PC's memory, mapped.
    counts: (ID3D12Resource, *mut u32, u64),
    /// The check's copies.
    scratch: OnceLock<Option<(ID3D12Resource, u64)>>,
}

// SAFETY: D3D12 objects are free-threaded; the mapping is only read.
unsafe impl Send for Work {}
// SAFETY: as above.
unsafe impl Sync for Work {}

fn work() -> Option<&'static Work> {
    static WORK: OnceLock<Option<Work>> = OnceLock::new();
    WORK.get_or_init(|| match make_work() {
        Ok(work) => Some(work),
        Err(e) => {
            tracing::warn!(error = %e, "VICTORY_BVH_COPY: our passes could not be made; the copy runs translated");
            None
        }
    })
    .as_ref()
}

fn make_work() -> windows::core::Result<Work> {
    let none = windows::core::Error::empty;
    let host = crate::host::host().ok_or_else(none)?;
    let copy = pass(&host.device, COPY)?;
    let compare = pass(&host.device, COMPARE)?;
    let records = host.uav_buffer(u64::from(RECORDS) * 16).ok_or_else(none)?;
    crate::host::name(&records, || "structure copy records".into());
    let (counts, data) = host.shared_buffer(64).ok_or_else(none)?;
    crate::host::name(&counts, || "structure copy counts".into());
    // SAFETY: a fresh mapping of 64 bytes; live buffers.
    let (records_gpu, counts_gpu) = unsafe {
        std::ptr::write_bytes(data, 0, 64);
        (
            records.GetGPUVirtualAddress(),
            counts.GetGPUVirtualAddress(),
        )
    };
    Ok(Work {
        copy,
        compare,
        records,
        records_gpu,
        counts: (counts, data.cast(), counts_gpu),
        scratch: OnceLock::new(),
    })
}

impl Work {
    fn scratch(&self) -> Option<u64> {
        self.scratch
            .get_or_init(|| {
                let buffer = crate::host::host()?.uav_buffer(SCRATCH)?;
                crate::host::name(&buffer, || "structure copy check".into());
                // SAFETY: a live buffer.
                let gpu = unsafe { buffer.GetGPUVirtualAddress() };
                Some((buffer, gpu))
            })
            .as_ref()
            .map(|s| s.1)
    }

    /// The counters as the card last left them.
    fn counts(&self) -> [u32; count::ALL] {
        // SAFETY: inside the 64-byte mapping; the card may be adding.
        std::array::from_fn(|i| unsafe { self.counts.1.add(i).read_volatile() })
    }
}

/// Where the program's accesses would have left notes, as our copy's
/// constants take them: byte offsets in the notes. Our translated programs
/// leave none (no touch or page marks in this fork's `crate::memory`), so
/// ours is given none either ([`Notes::default`]); the copy's shader keeps
/// the way to, as at the source.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct Notes {
    pub(super) marks: u32,
    pub(super) skew: u32,
    pub(super) src_touch: u32,
    pub(super) src_page: u32,
    pub(super) dst_touch: u32,
    pub(super) dst_page: u32,
}

/// One copy of ours: GPU addresses of the views, the counters and the
/// notes (the counters when there are none), and its constants.
pub(super) struct CopyArgs {
    pub(super) src: u64,
    pub(super) dst: u64,
    pub(super) records: u64,
    pub(super) counts: u64,
    pub(super) notes: u64,
    pub(super) constants: [u32; 10],
    pub(super) groups: [u32; 3],
}

/// The copy's constants (see [`COPY`]).
pub(super) fn constants(
    threads: u32,
    limit: u32,
    record: u32,
    groups: [u32; 3],
    notes: Notes,
) -> [u32; 10] {
    [
        threads,
        limit,
        record,
        groups[0].saturating_mul(32),
        notes.marks,
        notes.skew,
        notes.src_touch,
        notes.src_page,
        notes.dst_touch,
        notes.dst_page,
    ]
}

/// Records our copy in `l`: our root signature and pipeline, then the
/// dispatch. The list's root signature and pipeline are ours after it.
pub(super) fn record_copy(l: &ID3D12GraphicsCommandList, pass: &Pass, c: &CopyArgs) {
    // SAFETY: an open list; our root signature and pipeline; the addresses
    // are in live buffers.
    unsafe {
        l.SetComputeRootSignature(&pass.root);
        l.SetPipelineState(&pass.pso);
        l.SetComputeRoot32BitConstants(0, 10, c.constants.as_ptr().cast(), 0);
        l.SetComputeRootUnorderedAccessView(1, c.src);
        l.SetComputeRootUnorderedAccessView(2, c.dst);
        l.SetComputeRootUnorderedAccessView(3, c.records);
        l.SetComputeRootUnorderedAccessView(4, c.counts);
        l.SetComputeRootUnorderedAccessView(5, c.notes);
        l.Dispatch(c.groups[0], c.groups[1], c.groups[2]);
    }
}

/// Records our copy in `l` with what goes around it: in native mode
/// (`predicate`) the records move from the common state to unordered
/// access before it and on to predication after it ([`predicated`] moves
/// them back); in the check they stay in unordered access (implicitly).
/// Either way a barrier after it holds what follows until it is done.
pub(super) fn record_ours(
    l: &ID3D12GraphicsCommandList,
    pass: &Pass,
    c: &CopyArgs,
    records: &ID3D12Resource,
    predicate: bool,
) {
    let to = |before, after| crate::host::transition(records, before, after);
    // SAFETY: an open list; the records are in the common state in native
    // mode (no list leaves them otherwise).
    unsafe {
        if predicate {
            l.ResourceBarrier(&[to(
                D3D12_RESOURCE_STATE_COMMON,
                D3D12_RESOURCE_STATE_UNORDERED_ACCESS,
            )]);
        }
        record_copy(l, pass, c);
        if predicate {
            l.ResourceBarrier(&[
                uav_barrier(),
                to(
                    D3D12_RESOURCE_STATE_UNORDERED_ACCESS,
                    D3D12_RESOURCE_STATE_PREDICATION,
                ),
            ]);
        } else {
            l.ResourceBarrier(&[uav_barrier()]);
        }
    }
}

/// The dispatch of whatever `l` has bound, `[x, y, z]` groups, predicated
/// on the record at byte `record` of `records` (after [`record_ours`] in
/// native mode): it runs only if ours did not copy. The records go back
/// to the common state.
pub(super) fn predicated(
    l: &ID3D12GraphicsCommandList,
    records: &ID3D12Resource,
    record: u32,
    [x, y, z]: [u32; 3],
) {
    // SAFETY: an open list; the records are in the predication state.
    unsafe {
        l.SetPredication(records, u64::from(record), D3D12_PREDICATION_OP_EQUAL_ZERO);
        l.Dispatch(x, y, z);
        l.SetPredication(None::<&ID3D12Resource>, 0, D3D12_PREDICATION_OP_EQUAL_ZERO);
        l.ResourceBarrier(&[crate::host::transition(
            records,
            D3D12_RESOURCE_STATE_PREDICATION,
            D3D12_RESOURCE_STATE_COMMON,
        )]);
    }
}

/// Records the check's compare in `l` of `ours` and `theirs` over the
/// length record `record` holds.
pub(super) fn record_compare(
    l: &ID3D12GraphicsCommandList,
    pass: &Pass,
    [ours, theirs, records, counts]: [u64; 4],
    record: u32,
) {
    let constants = [record, COMPARE_GROUPS * 64];
    // SAFETY: as in `record_copy`.
    unsafe {
        l.SetComputeRootSignature(&pass.root);
        l.SetPipelineState(&pass.pso);
        l.SetComputeRoot32BitConstants(0, 2, constants.as_ptr().cast(), 0);
        l.SetComputeRootUnorderedAccessView(1, ours);
        l.SetComputeRootUnorderedAccessView(2, theirs);
        l.SetComputeRootUnorderedAccessView(3, records);
        l.SetComputeRootUnorderedAccessView(4, counts);
        l.Dispatch(COMPARE_GROUPS, 1, 1);
    }
}

pub(super) fn uav_barrier() -> D3D12_RESOURCE_BARRIER {
    D3D12_RESOURCE_BARRIER {
        Type: D3D12_RESOURCE_BARRIER_TYPE_UAV,
        Flags: D3D12_RESOURCE_BARRIER_FLAG_NONE,
        Anonymous: D3D12_RESOURCE_BARRIER_0 {
            UAV: ManuallyDrop::new(D3D12_RESOURCE_UAV_BARRIER {
                pResource: ManuallyDrop::new(None),
            }),
        },
    }
}

/// The title root parameters of the program's source, destination and
/// constants (from its used-resource table), if the program and the
/// dispatch are as ours expects: groups of 32 threads, and whole groups.
pub(super) fn slots(
    used: &[UsedResource],
    threads: [u32; 3],
    partial: [u32; 3],
) -> Result<(u8, u8, u8), &'static str> {
    if threads != [32, 1, 1] {
        return Err("its groups are not of 32 threads");
    }
    if partial != [0; 3] {
        return Err("a dispatch of threads that ends inside a group");
    }
    let slot = |kind: ResourceType, root: RootType| {
        let mut found = used.iter().filter(|u| u.kind == kind && u.root == root);
        match (found.next(), found.next()) {
            (Some(u), None) => Some(u.slot),
            _ => None,
        }
    };
    match (
        slot(ResourceType::Srv, RootType::Descriptor),
        slot(ResourceType::Uav, RootType::Descriptor),
        used.iter()
            .find(|u| u.root == RootType::Constants)
            .map(|u| u.slot),
    ) {
        (Some(src), Some(dst), Some(constants)) => Ok((src, dst, constants)),
        _ => Err("its views are not one source and one destination with constants"),
    }
}

/// A view as ours takes it: its title address, the host GPU address of
/// that byte, and how many bytes from there ours may reach.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Side {
    pub(super) address: u64,
    pub(super) gpu: u64,
    pub(super) reach: u64,
}

/// The most bytes ours may copy from `src` to `dst` for constants `c0` and
/// `c1` (`room`: the most the destination holds besides its reach), or why
/// the translated program runs instead.
pub(super) fn limit(
    c0: u32,
    c1: u32,
    src: Side,
    dst: Side,
    room: u64,
) -> Result<u32, &'static str> {
    if c1 != 0 {
        return Err("it also writes bounds (constant 1 is set)");
    }
    // The program's step, in its 32-bit arithmetic.
    let step = c0 << 4;
    if step == 0 {
        return Err("its step is 0 (constant 0)");
    }
    if !(src.address | dst.address | src.gpu | dst.gpu).is_multiple_of(16) {
        return Err("a view not on 16 bytes");
    }
    if src.address == dst.address {
        return Err("the source is the destination");
    }
    if src.reach < 8 {
        return Err("the source's header is not in its copy");
    }
    // Ranges that overlap in title memory would give a result that depends
    // on the order the program's threads ran in.
    let gap = src.address.abs_diff(dst.address);
    let limit = src
        .reach
        .min(dst.reach)
        .min(room)
        .min(gap)
        .min(u64::from(step) * ROUNDS)
        .min(u64::from(u32::MAX));
    if limit < 64 {
        return Err("not even the shortest copy fits");
    }
    Ok(limit as u32)
}

/// The view at title address `address` as ours takes it ([`Side`]), with
/// the number the directory gives its copy and the GPU address of the
/// notes (0 and 0 for a program that does not read title memory by
/// address). The list keeps the copy's buffer.
fn side(list: &HostList, address: u64, by_address: bool, write: bool) -> Result<(Side, u32, u64), &'static str> {
    let shadow =
        crate::shadow::find(address, 1).ok_or("a view outside the title's buffers and memory")?;
    // SAFETY: a live buffer.
    let (flags, base) = unsafe {
        (
            shadow.buffer.GetDesc().Flags,
            shadow.buffer.GetGPUVirtualAddress(),
        )
    };
    if flags.0 & D3D12_RESOURCE_FLAG_ALLOW_UNORDERED_ACCESS.0 == 0 {
        return Err("a copy shaders cannot write");
    }
    // The directory's views of a copy start at its buffer's first byte.
    if shadow.gpu(shadow.span().0) != base {
        return Err("a copy in place that starts inside its buffer");
    }
    let most = shadow.left(address).min(MOST);
    let (reach, copy, notes) = if by_address {
        crate::memory::run_in(&shadow, address, most, write)
            .ok_or("a page the directory gives no copy or another one")?
    } else {
        (most, 0, 0)
    };
    list.keep(&shadow.buffer);
    Ok((
        Side {
            address,
            gpu: shadow.gpu(address),
            reach,
        },
        copy,
        notes,
    ))
}

/// What ours recorded ahead of the program's bind.
pub(super) struct Recorded {
    mode: Mode,
    record: u32,
    /// The destination's GPU address, and where ours copied to (the
    /// check's buffer in the check).
    dst: u64,
    to: u64,
}

/// Counted here since the start: dispatches of the program, the ones ours
/// was recorded for, the ones left to the program at recording.
static TALLY: [AtomicU64; 3] = [const { AtomicU64::new(0) }; 3];

impl State {
    /// Before a dispatch's program is bound: for the builder's copy with
    /// `VICTORY_BVH_COPY` set, ours is recorded (the list's root signature
    /// and pipeline are ours after it, and the next bind sets the title's
    /// again). None if ours is not wanted or not possible: the program
    /// runs as before.
    pub(super) fn bvh_copy_before(
        &mut self,
        list: &HostList,
        p: &HostPipeline,
        groups: [u32; 3],
        partial: [u32; 3],
    ) -> Option<Recorded> {
        let mode = mode()?;
        let stage = p.stages.first()?;
        if stage.shader.hash != HASH {
            return None;
        }
        frame_line();
        TALLY[0].fetch_add(1, Ordering::Relaxed);
        match self.bvh_copy_record(list, stage, groups, partial, mode) {
            Ok(recorded) => {
                TALLY[1].fetch_add(1, Ordering::Relaxed);
                Some(recorded)
            }
            Err(why) => {
                fell_back(why);
                None
            }
        }
    }

    fn bvh_copy_record(
        &mut self,
        list: &HostList,
        stage: &Stage,
        groups: [u32; 3],
        partial: [u32; 3],
        mode: Mode,
    ) -> Result<Recorded, &'static str> {
        let t = stage.translated();
        let (src_slot, dst_slot, constants_slot) = slots(&t.used, t.threads.ok_or("its group size is not known")?, partial)?;
        let roots = self.roots_ref(true);
        let view = |slot: u8| match roots.get(usize::from(slot)) {
            Some(&Root::View(address)) => Ok(address),
            _ => Err("a view the title did not set"),
        };
        let (src, dst) = (view(src_slot)?, view(dst_slot)?);
        let Some(Root::Constants(words)) = roots.get(usize::from(constants_slot)) else {
            return Err("constants the title did not set");
        };
        let (c0, c1) = (
            words.first().copied().unwrap_or(0),
            words.get(1).copied().unwrap_or(0),
        );
        let by_address = t
            .host
            .root_srvs
            .contains(&victory_shader::emit::SLOT_MEMORY);
        let work = work().ok_or("our passes could not be made")?;
        let (src_side, _, _) = side(list, src, by_address, false)?;
        let (dst_side, _, _) = side(list, dst, by_address, true)?;
        let check = mode == Mode::Check;
        let (to, room) = if check {
            (work.scratch().ok_or("no buffer for the check")?, SCRATCH)
        } else {
            (dst_side.gpu, u64::MAX)
        };
        let limit = limit(c0, c1, src_side, dst_side, room)?;
        let record = next_record();
        // The program's notes: none in this fork (see [`Notes`]).
        let (noted, notes) = (Notes::default(), work.counts.2);
        let copy = CopyArgs {
            src: src_side.gpu,
            dst: to,
            records: work.records_gpu,
            counts: work.counts.2,
            notes,
            constants: constants(c0, limit, record, groups, noted),
            groups,
        };
        #[cfg(feature = "diagnostics")]
        list.note_op(|| {
            format!("structure copy (ours, VICTORY_BVH_COPY) {groups:?} to reach {limit}")
        });
        record_ours(&list.list, &work.copy, &copy, &work.records, !check);
        // Ours replaced the program's root signature and pipeline: the bind
        // sets them again.
        self.host_roots[1] = 0;
        self.host_pipeline = 0;
        Ok(Recorded {
            mode,
            record,
            dst: dst_side.gpu,
            to,
        })
    }

    /// The program is bound after ours ([`State::bvh_copy_before`]): its
    /// dispatch, predicated on our record (native: it runs only where ours
    /// did not copy), or as before and then compared (check).
    pub(super) fn bvh_copy_after(&mut self, list: &HostList, ours: &Recorded, [x, y, z]: [u32; 3]) {
        let Some(work) = work() else {
            return;
        };
        let l = &list.list;
        match ours.mode {
            Mode::Native => predicated(l, &work.records, ours.record, [x, y, z]),
            Mode::Check => {
                // SAFETY: an open list with the program bound.
                unsafe {
                    l.Dispatch(x, y, z);
                    l.ResourceBarrier(&[uav_barrier()]);
                }
                #[cfg(feature = "diagnostics")]
                list.note_op(|| "structure copy check (ours, VICTORY_BVH_COPY=check)".to_owned());
                record_compare(
                    l,
                    &work.compare,
                    [ours.to, ours.dst, work.records_gpu, work.counts.2],
                    ours.record,
                );
                // SAFETY: an open list.
                unsafe { l.ResourceBarrier(&[uav_barrier()]) };
                self.host_roots[1] = 0;
                self.host_pipeline = 0;
            }
        }
    }

    /// The program could not be bound after ours ran: the records go back
    /// to the common state.
    pub(super) fn bvh_copy_dropped(&self, list: &HostList, ours: &Recorded) {
        if let (Mode::Native, Some(work)) = (ours.mode, work()) {
            // SAFETY: an open list; the records are in the predication state.
            unsafe {
                list.list.ResourceBarrier(&[crate::host::transition(
                    &work.records,
                    D3D12_RESOURCE_STATE_PREDICATION,
                    D3D12_RESOURCE_STATE_COMMON,
                )])
            };
        }
    }
}

/// The byte offset of the next record.
fn next_record() -> u32 {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    (NEXT.fetch_add(1, Ordering::Relaxed) % RECORDS) * 16
}

/// The program runs translated for `why` (each reason logged once).
fn fell_back(why: &'static str) {
    TALLY[2].fetch_add(1, Ordering::Relaxed);
    static SAID: Mutex<Vec<&'static str>> = Mutex::new(Vec::new());
    let mut said = SAID.lock().unwrap_or_else(|e| e.into_inner());
    if !said.contains(&why) {
        said.push(why);
        tracing::info!(
            why,
            "VICTORY_BVH_COPY: a structure copy runs translated (logged once per reason)"
        );
    }
}

/// At the first copy of a frame: a line for the frames since the last one,
/// with the counts here and the card's (which lag by the frames in
/// flight). The first 200 lines, then every 100th.
fn frame_line() {
    static FRAME: AtomicU64 = AtomicU64::new(u64::MAX);
    static BEFORE: Mutex<[u64; 3 + count::ALL]> = Mutex::new([0; 3 + count::ALL]);
    static LINES: AtomicU32 = AtomicU32::new(0);
    let frame = crate::frame::current();
    if FRAME.swap(frame, Ordering::Relaxed) == frame {
        return;
    }
    // Read under the lock: a line for a later frame on another thread
    // cannot come between.
    let mut before = BEFORE.lock().unwrap_or_else(|e| e.into_inner());
    let card = work().map_or([0; count::ALL], Work::counts);
    let mut now = [0u64; 3 + count::ALL];
    for (i, t) in TALLY.iter().enumerate() {
        now[i] = t.load(Ordering::Relaxed);
    }
    for (i, &c) in card.iter().enumerate() {
        now[3 + i] = u64::from(c);
    }
    // The card's counters are 32-bit and wrap.
    let d: Vec<u64> = (0..now.len())
        .map(|i| {
            if i < 3 {
                now[i].saturating_sub(before[i])
            } else {
                u64::from((now[i] as u32).wrapping_sub(before[i] as u32))
            }
        })
        .collect();
    *before = now;
    drop(before);
    if d[0] == 0 {
        return;
    }
    let line = LINES.fetch_add(1, Ordering::Relaxed);
    if line >= 200 && !line.is_multiple_of(100) {
        return;
    }
    tracing::info!(
        frame,
        dispatches = d[0],
        ours = d[1],
        translated = d[2],
        copied = d[3 + count::DONE / 4],
        too_long = d[3 + count::OVER / 4],
        compared = d[3 + count::COMPARED / 4],
        compared_kib = d[3 + count::DWORDS / 4] / 256,
        differing_dwords = d[3 + count::DIFFERING / 4],
        differing_copies = d[3 + count::COPIES_DIFFERING / 4],
        not_compared = d[3 + count::NOT_COMPARED / 4],
        "VICTORY_BVH_COPY, since the last line: the structure builder's copies (ours: recorded by us; translated: left to the program at recording; the card's counts lag a frame or two)"
    );
}

#[cfg(test)]
#[path = "bvh_copy_tests.rs"]
mod tests;
