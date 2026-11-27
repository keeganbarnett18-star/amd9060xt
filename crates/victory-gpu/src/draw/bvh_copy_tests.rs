//! Tests of the structure builder's copy by a pass of ours: the decisions
//! that leave a dispatch to the translated program, a model of the title's
//! program on the CPU, and our passes on the software adapter (always run)
//! and on the GPU (ignored: `cargo test -p victory-gpu --lib bvh_copy --
//! --ignored`).

use super::*;
use windows::Win32::Foundation::HANDLE;
use windows::Win32::Graphics::Direct3D::D3D_FEATURE_LEVEL_12_0;
use windows::Win32::Graphics::Dxgi::Common::DXGI_SAMPLE_DESC;
use windows::Win32::Graphics::Dxgi::*;
use windows::Win32::System::Threading::{CreateEventW, INFINITE, WaitForSingleObject};
use windows::core::Interface;

/// What the title's program leaves in `dst` for the source `src` (both
/// from the views' first bytes), constant 0 `c0` and `lanes` threads along
/// x, as its code does it: 32-bit sums, no cap on the loop.
fn title(src: &[u8], dst: &mut [u8], c0: u32, lanes: u32) {
    let bytes = (u32::from_le_bytes(src[4..8].try_into().unwrap()) << 6).wrapping_add(64);
    let step = c0 << 4;
    assert_ne!(step, 0, "the title's program would not end");
    for thread in 0..lanes {
        let mut at = thread << 4;
        while at < bytes {
            let a = at as usize;
            dst[a..a + 16].copy_from_slice(&src[a..a + 16]);
            match at.checked_add(step) {
                Some(next) => at = next,
                None => break,
            }
        }
    }
}

/// Bytes from a fixed seed (xorshift).
fn noise(len: usize, mut seed: u64) -> Vec<u8> {
    (0..len)
        .map(|_| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed >> 24) as u8
        })
        .collect()
}

/// A source of `len` bytes whose view at `at` asks for a copy of `bytes`.
fn source(len: usize, at: usize, bytes: u32, seed: u64) -> Vec<u8> {
    let mut src = noise(len, seed);
    src[at + 4..at + 8].copy_from_slice(&(bytes / 64 - 1).to_le_bytes());
    src
}

fn side(address: u64, reach: u64) -> Side {
    Side {
        address,
        gpu: 0x7_0000_0000 + address,
        reach,
    }
}

#[test]
fn the_model_copies_what_the_header_asks_for() {
    let src = source(4096, 0, 1024, 1);
    let mut dst = vec![0xCD; 4096];
    title(&src, &mut dst, 32, 32);
    assert_eq!(dst[..1024], src[..1024]);
    assert!(dst[1024..].iter().all(|&b| b == 0xCD));
    // A step past the threads leaves chunks out, as the program does.
    let mut dst = vec![0xCD; 4096];
    title(&src, &mut dst, 64, 32);
    assert_eq!(dst[..512], src[..512]);
    assert!(dst[512..1024].iter().all(|&b| b == 0xCD));
}

#[test]
fn ours_reaches_as_far_as_both_copies_and_their_distance() {
    let (src, dst) = (side(0x10_0000, 4 << 20), side(0x80_0000, 1 << 20));
    assert_eq!(limit(0x1000, 0, src, dst, u64::MAX), Ok(1 << 20));
    assert_eq!(limit(0x1000, 0, dst, src, u64::MAX), Ok(1 << 20));
    // The check's buffer.
    assert_eq!(
        limit(0x1000, 0, src, dst, SCRATCH.min(64 << 10)),
        Ok(64 << 10)
    );
    // Ranges that would overlap in title memory, either way round.
    let (a, b) = (side(0x10_0000, 4 << 20), side(0x10_1000, 4 << 20));
    assert_eq!(limit(0x1000, 0, a, b, u64::MAX), Ok(0x1000));
    assert_eq!(limit(0x1000, 0, b, a, u64::MAX), Ok(0x1000));
    // A tiny step: 65,536 rounds of 16 bytes at most.
    assert_eq!(limit(1, 0, src, dst, u64::MAX), Ok(1 << 20));
    assert_eq!(
        limit(
            1,
            0,
            side(0x1000_0000, 8 << 20),
            side(0x2000_0000, 8 << 20),
            u64::MAX
        ),
        Ok(1 << 20)
    );
    // 32-bit lengths.
    let far = (side(0x1_0000_0000, 1 << 40), side(0x200_0000_0000, 1 << 40));
    assert_eq!(limit(0x4000, 0, far.0, far.1, u64::MAX), Ok(u32::MAX));
}

#[test]
fn dispatches_ours_cannot_copy_exactly_run_translated() {
    let (src, dst) = (side(0x10_0000, 1 << 20), side(0x80_0000, 1 << 20));
    assert!(limit(0x1000, 0, src, dst, u64::MAX).is_ok());
    // It also writes bounds.
    assert!(limit(0x1000, 1, src, dst, u64::MAX).is_err());
    // A step of 0, also one that wraps to 0 in 32 bits.
    assert!(limit(0, 0, src, dst, u64::MAX).is_err());
    assert!(limit(1 << 28, 0, src, dst, u64::MAX).is_err());
    // Views not on 16 bytes, in title memory or on the host.
    assert!(limit(0x1000, 0, side(0x10_0008, 1 << 20), dst, u64::MAX).is_err());
    assert!(
        limit(
            0x1000,
            0,
            src,
            Side {
                gpu: dst.gpu + 4,
                ..dst
            },
            u64::MAX
        )
        .is_err()
    );
    // The source is the destination.
    assert!(limit(0x1000, 0, src, src, u64::MAX).is_err());
    // The header is not in the source's copy.
    assert!(limit(0x1000, 0, side(0x10_0000, 4), dst, u64::MAX).is_err());
    // Not even 64 bytes: the copy's reach, the distance, the check's room.
    assert!(limit(0x1000, 0, src, side(0x80_0000, 48), u64::MAX).is_err());
    assert!(limit(0x1000, 0, src, side(0x10_0020, 1 << 20), u64::MAX).is_err());
    assert!(limit(0x1000, 0, src, dst, 32).is_err());
}

fn used(kind: ResourceType, root: RootType, slot: u8) -> UsedResource {
    UsedResource {
        kind,
        root,
        table_offset_dwords: 0,
        lower_bound: 0,
        stride: 1,
        dwords: 4,
        user_data: 16 + 2 * slot,
        slot,
    }
}

#[test]
fn ours_takes_the_programs_views_from_its_table() {
    let table = [
        used(ResourceType::Srv, RootType::Descriptor, 0),
        used(ResourceType::Uav, RootType::Descriptor, 1),
        used(ResourceType::Cbv, RootType::Constants, 2),
    ];
    assert_eq!(slots(&table, [32, 1, 1], [0; 3]), Ok((0, 1, 2)));
    // Other groups, a dispatch of threads that ends inside a group.
    assert!(slots(&table, [64, 1, 1], [0; 3]).is_err());
    assert!(slots(&table, [32, 1, 1], [100, 0, 0]).is_err());
    // Another table: no destination, two sources, no constants.
    assert!(slots(&table[..1], [32, 1, 1], [0; 3]).is_err());
    let two = [
        table[0],
        used(ResourceType::Srv, RootType::Descriptor, 3),
        table[1],
        table[2],
    ];
    assert!(slots(&two, [32, 1, 1], [0; 3]).is_err());
    assert!(slots(&table[..2], [32, 1, 1], [0; 3]).is_err());
}

#[test]
fn the_switch_takes_native_and_check_only() {
    assert_eq!(parse_mode("native"), Some(Mode::Native));
    assert_eq!(parse_mode(" check "), Some(Mode::Check));
    assert_eq!(parse_mode(""), None);
    assert_eq!(parse_mode("1"), None);
    assert_eq!(parse_mode("Native"), None);
}

/// A device with one list, on the software adapter or the first hardware
/// one.
struct Gpu {
    device: ID3D12Device,
    queue: ID3D12CommandQueue,
    allocator: ID3D12CommandAllocator,
    list: ID3D12GraphicsCommandList,
    fence: ID3D12Fence,
    done: u64,
    event: HANDLE,
}

/// Our tests' devices, one per adapter for the whole run, and one test on
/// them at a time: WARP devices made and let go on several threads at
/// once crash the process now and then (the older WARP tests of this
/// crate do too, about one parallel run in twenty; never one at a time).
struct Shared(ID3D12Device);

// SAFETY: D3D12 devices are free-threaded.
unsafe impl Send for Shared {}
// SAFETY: as above.
unsafe impl Sync for Shared {}

static ONE: Mutex<()> = Mutex::new(());

fn device(warp: bool) -> Option<ID3D12Device> {
    static DEVICES: [OnceLock<Option<Shared>>; 2] = [const { OnceLock::new() }; 2];
    let made = DEVICES[usize::from(warp)].get_or_init(|| {
        // SAFETY: plain adapter queries and device creation.
        unsafe {
            let factory: IDXGIFactory4 = CreateDXGIFactory2(DXGI_CREATE_FACTORY_FLAGS(0)).ok()?;
            let software = |a: &IDXGIAdapter1| {
                a.GetDesc1()
                    .is_ok_and(|d| d.Flags & DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32 != 0)
            };
            let adapter: IDXGIAdapter1 = if warp {
                factory.EnumWarpAdapter().ok()?
            } else {
                (0..)
                    .map_while(|i| factory.EnumAdapters1(i).ok())
                    .find(|a| !software(a))?
            };
            let mut device: Option<ID3D12Device> = None;
            D3D12CreateDevice(&adapter, D3D_FEATURE_LEVEL_12_0, &mut device).ok()?;
            device.map(Shared)
        }
    });
    made.as_ref().map(|d| d.0.clone())
}

impl Gpu {
    fn new(warp: bool) -> Option<Gpu> {
        let device = device(warp)?;
        // SAFETY: plain object creation.
        unsafe {
            let queue: ID3D12CommandQueue = device
                .CreateCommandQueue(&D3D12_COMMAND_QUEUE_DESC {
                    Type: D3D12_COMMAND_LIST_TYPE_DIRECT,
                    ..Default::default()
                })
                .ok()?;
            let allocator: ID3D12CommandAllocator = device
                .CreateCommandAllocator(D3D12_COMMAND_LIST_TYPE_DIRECT)
                .ok()?;
            let list = device
                .CreateCommandList(0, D3D12_COMMAND_LIST_TYPE_DIRECT, &allocator, None)
                .ok()?;
            let fence = device.CreateFence(0, D3D12_FENCE_FLAG_NONE).ok()?;
            let event = CreateEventW(None, false, false, None).ok()?;
            Some(Gpu {
                device,
                queue,
                allocator,
                list,
                fence,
                done: 0,
                event,
            })
        }
    }

    /// A buffer shaders write: in the PC's memory and mapped as the host's
    /// copies are (`mapped`), or in the card's own as the records are.
    fn buffer(&self, size: usize, mapped: bool) -> (ID3D12Resource, *mut u8, u64) {
        let heap = if mapped {
            D3D12_HEAP_PROPERTIES {
                Type: D3D12_HEAP_TYPE_CUSTOM,
                CPUPageProperty: D3D12_CPU_PAGE_PROPERTY_WRITE_BACK,
                MemoryPoolPreference: D3D12_MEMORY_POOL_L0,
                ..Default::default()
            }
        } else {
            D3D12_HEAP_PROPERTIES {
                Type: D3D12_HEAP_TYPE_DEFAULT,
                ..Default::default()
            }
        };
        let desc = D3D12_RESOURCE_DESC {
            Dimension: D3D12_RESOURCE_DIMENSION_BUFFER,
            Width: size as u64,
            Height: 1,
            DepthOrArraySize: 1,
            MipLevels: 1,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Layout: D3D12_TEXTURE_LAYOUT_ROW_MAJOR,
            Flags: D3D12_RESOURCE_FLAG_ALLOW_UNORDERED_ACCESS,
            ..Default::default()
        };
        let mut buffer: Option<ID3D12Resource> = None;
        let mut data = std::ptr::null_mut();
        // SAFETY: plain creation and mapping of a buffer.
        unsafe {
            self.device
                .CreateCommittedResource(
                    &heap,
                    D3D12_HEAP_FLAG_NONE,
                    &desc,
                    D3D12_RESOURCE_STATE_COMMON,
                    None,
                    &mut buffer,
                )
                .unwrap();
            let buffer = buffer.unwrap();
            if mapped {
                buffer.Map(0, None, Some(&mut data)).unwrap();
                std::ptr::write_bytes(data.cast::<u8>(), 0, size);
            }
            let gpu = buffer.GetGPUVirtualAddress();
            (buffer, data.cast(), gpu)
        }
    }

    /// Runs the list and waits for it.
    fn run(&mut self) {
        self.done += 1;
        // SAFETY: our list, queue and fence; the list is not running.
        unsafe {
            self.list.Close().unwrap();
            self.queue
                .ExecuteCommandLists(&[Some(self.list.cast().unwrap())]);
            self.queue.Signal(&self.fence, self.done).unwrap();
            self.fence
                .SetEventOnCompletion(self.done, self.event)
                .unwrap();
            WaitForSingleObject(self.event, INFINITE);
            self.device.GetDeviceRemovedReason().unwrap();
            self.allocator.Reset().unwrap();
            self.list.Reset(&self.allocator, None).unwrap();
        }
    }

    /// Record `at` of `records` (in the card's memory), through `readback`.
    fn record(
        &mut self,
        records: &ID3D12Resource,
        at: u32,
        readback: &(ID3D12Resource, *mut u8, u64),
    ) -> [u32; 4] {
        // SAFETY: our open list; live buffers of at least 16 bytes there.
        unsafe {
            self.list
                .CopyBufferRegion(&readback.0, 0, records, u64::from(at), 16)
        };
        self.run();
        words(readback.1, 4).try_into().unwrap()
    }
}

impl Drop for Gpu {
    fn drop(&mut self) {
        // SAFETY: our own event, waited on no more.
        let _ = unsafe { windows::Win32::Foundation::CloseHandle(self.event) };
    }
}

/// `n` dwords at `data`.
fn words(data: *const u8, n: usize) -> Vec<u32> {
    // SAFETY: the callers' mappings hold them.
    (0..n)
        .map(|i| unsafe { data.cast::<u32>().add(i).read_volatile() })
        .collect()
}

fn bytes_at(data: *const u8, len: usize) -> Vec<u8> {
    // SAFETY: the callers' mappings hold them.
    unsafe { std::slice::from_raw_parts(data, len).to_vec() }
}

fn put(data: *mut u8, bytes: &[u8]) {
    // SAFETY: the callers' mappings hold them.
    unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), data, bytes.len()) };
}

/// Bytes after each view.
const TAIL: usize = 4096;

/// Ours copies what the title's program copies, byte for byte, and
/// nothing past it: lengths of 64 bytes, 64 KiB less and more 64, 1 MiB
/// and 64; views at offsets in their buffers; the title's thread counts,
/// and steps past and short of the threads dispatched.
fn copies_as_the_title(gpu: &mut Gpu) {
    let copy = pass(&gpu.device, COPY).expect("our copy compiles");
    let (records, _, records_gpu) = gpu.buffer(64 * 16, false);
    let (_counts, counts, counts_gpu) = gpu.buffer(64, true);
    let readback = gpu.buffer(16, true);
    // (bytes, source view, destination view, c0, groups)
    let cases: [(u32, usize, usize, u32, u32); 8] = [
        (64, 0x10, 0x20, 0x1000, 128),
        ((64 << 10) - 64, 0x1230, 0x750, 0x1000, 128),
        ((64 << 10) + 64, 0x1230, 0x750, 0x1000, 128),
        ((1 << 20) + 64, 0x40, 0x9f0, 0x1000, 128),
        ((64 << 10) + 64, 0x40, 0x40, 0x4000, 512),
        ((1 << 20) + 64, 0x80, 0x90, 0x4000, 512),
        // Steps past the threads dispatched (chunks left out) and short of
        // them (chunks copied twice).
        ((1 << 20) + 64, 0x80, 0x90, 0x4000, 128),
        ((64 << 10) + 64, 0x80, 0x90, 0x400, 128),
    ];
    for (i, &(bytes, src_at, dst_at, c0, groups)) in cases.iter().enumerate() {
        let n = bytes as usize;
        let (src_len, dst_len) = (src_at + n + TAIL, dst_at + n + TAIL);
        let (_s, s, s_gpu) = gpu.buffer(src_len, true);
        let (_d, d, d_gpu) = gpu.buffer(dst_len, true);
        let src = source(src_len, src_at, bytes, 0x9e37_79b9_7f4a_7c15 ^ i as u64);
        put(s, &src);
        put(d, &vec![0xCD; dst_len]);
        let limit = (src_len - src_at).min(dst_len - dst_at) as u32;
        let record = 16 * i as u32;
        let args = CopyArgs {
            src: s_gpu + src_at as u64,
            dst: d_gpu + dst_at as u64,
            records: records_gpu,
            counts: counts_gpu,
            notes: counts_gpu,
            constants: constants(c0, limit, record, [groups, 1, 1], Notes::default()),
            groups: [groups, 1, 1],
        };
        record_ours(&gpu.list, &copy, &args, &records, false);
        gpu.run();
        let mut expected = vec![0xCD; dst_len];
        title(&src[src_at..], &mut expected[dst_at..], c0, groups * 32);
        let got = bytes_at(d, dst_len);
        let wrong = got.iter().zip(&expected).filter(|(a, b)| a != b).count();
        let first = got.iter().zip(&expected).position(|(a, b)| a != b);
        assert_eq!(
            wrong, 0,
            "case {i}: {wrong} bytes differ from the title's, the first at {first:?}"
        );
        assert!(
            got[dst_at + n..].iter().all(|&b| b == 0xCD),
            "case {i}: bytes past the copy changed"
        );
        assert!(
            got[..dst_at].iter().all(|&b| b == 0xCD),
            "case {i}: bytes before the copy changed"
        );
        assert_eq!(
            gpu.record(&records, record, &readback),
            [0, 0, bytes, 0],
            "case {i}: the record"
        );
    }
    let c = words(counts, count::ALL);
    assert_eq!(
        (c[count::DONE / 4], c[count::OVER / 4]),
        (cases.len() as u32, 0)
    );
}

/// A copy longer than ours may reach copies nothing; its record's flag is
/// set and it is counted.
fn too_long_copies_nothing(gpu: &mut Gpu) {
    let copy = pass(&gpu.device, COPY).expect("our copy compiles");
    let (records, _, records_gpu) = gpu.buffer(64 * 16, false);
    let (_counts, counts, counts_gpu) = gpu.buffer(64, true);
    let readback = gpu.buffer(16, true);
    let (bytes, at) = ((64u32 << 10) + 64, 0x40usize);
    let len = at + bytes as usize + TAIL;
    let (_s, s, s_gpu) = gpu.buffer(len, true);
    let (_d, d, d_gpu) = gpu.buffer(len, true);
    put(s, &source(len, at, bytes, 5));
    put(d, &vec![0xCD; len]);
    let args = CopyArgs {
        src: s_gpu + at as u64,
        dst: d_gpu + at as u64,
        records: records_gpu,
        counts: counts_gpu,
        notes: counts_gpu,
        constants: constants(0x1000, bytes - 64, 32, [128, 1, 1], Notes::default()),
        groups: [128, 1, 1],
    };
    record_ours(&gpu.list, &copy, &args, &records, false);
    gpu.run();
    assert!(bytes_at(d, len).iter().all(|&b| b == 0xCD));
    assert_eq!(gpu.record(&records, 32, &readback), [1, 0, bytes, 0]);
    let c = words(counts, count::ALL);
    assert_eq!((c[count::DONE / 4], c[count::OVER / 4]), (0, 1));
}

/// The notes the program's accesses would leave: the destination's page
/// marks (each 64 KiB page the copy writes), and both copies' touches.
fn leaves_the_programs_notes(gpu: &mut Gpu) {
    let copy = pass(&gpu.device, COPY).expect("our copy compiles");
    let (records, _, records_gpu) = gpu.buffer(64 * 16, false);
    let (_counts, _, counts_gpu) = gpu.buffer(64, true);
    let (_notes, notes, notes_gpu) = gpu.buffer(4096, true);
    let (bytes, at) = ((64u32 << 10) + 64, 0x40usize);
    let len = at + bytes as usize + TAIL;
    let (_s, s, s_gpu) = gpu.buffer(len, true);
    let (_d, _, d_gpu) = gpu.buffer(len, true);
    put(s, &source(len, at, bytes, 6));
    // The destination starts 64 bytes before the end of its page: the
    // copy writes two pages.
    let noted = Notes {
        marks: 256,
        skew: 0xFFC0,
        src_touch: 8,
        src_page: 0x777,
        dst_touch: 12,
        dst_page: 0x888,
    };
    let args = CopyArgs {
        src: s_gpu + at as u64,
        dst: d_gpu + at as u64,
        records: records_gpu,
        counts: counts_gpu,
        notes: notes_gpu,
        constants: constants(0x1000, bytes, 0, [128, 1, 1], noted),
        groups: [128, 1, 1],
    };
    record_ours(&gpu.list, &copy, &args, &records, false);
    gpu.run();
    let w = words(notes, 1024);
    assert_eq!(w[2..4], [0x777, 0x888]);
    assert_eq!(w[63..67], [0, 1, 1, 0]);
    assert_eq!(w.iter().filter(|&&v| v != 0).count(), 4);
}

/// Native mode on the card: the program's dispatch, predicated on our
/// record, is skipped where ours copied and runs where it did not.
fn the_program_runs_only_where_ours_did_not(gpu: &mut Gpu) {
    const MARKER: &str = "#define RS \"UAV(u0)\"\nRWByteAddressBuffer hits : register(u0);\n[RootSignature(RS)]\n[numthreads(1, 1, 1)]\nvoid main() { uint was; hits.InterlockedAdd(0, 1, was); }\n";
    let copy = pass(&gpu.device, COPY).expect("our copy compiles");
    let marker = pass(&gpu.device, MARKER).expect("the marker compiles");
    let (records, _, records_gpu) = gpu.buffer(64 * 16, false);
    let (_counts, _, counts_gpu) = gpu.buffer(64, true);
    let (_hits, hits, hits_gpu) = gpu.buffer(64, true);
    let (bytes, at) = ((64u32 << 10) + 64, 0x40usize);
    let len = at + bytes as usize + TAIL;
    let (_s, s, s_gpu) = gpu.buffer(len, true);
    let (_d, _, d_gpu) = gpu.buffer(len, true);
    put(s, &source(len, at, bytes, 7));
    // Twice in one list each time, as two dispatches of a title list.
    for (limit, runs) in [(bytes, 0), (bytes - 64, 2)] {
        put(hits, &[0; 4]);
        for record in [16, 48] {
            let args = CopyArgs {
                src: s_gpu + at as u64,
                dst: d_gpu + at as u64,
                records: records_gpu,
                counts: counts_gpu,
                notes: counts_gpu,
                constants: constants(0x1000, limit, record, [128, 1, 1], Notes::default()),
                groups: [128, 1, 1],
            };
            record_ours(&gpu.list, &copy, &args, &records, true);
            // SAFETY: our open list and pass; a live buffer.
            unsafe {
                gpu.list.SetComputeRootSignature(&marker.root);
                gpu.list.SetPipelineState(&marker.pso);
                gpu.list.SetComputeRootUnorderedAccessView(0, hits_gpu);
            }
            predicated(&gpu.list, &records, record, [1, 1, 1]);
        }
        gpu.run();
        assert_eq!(words(hits, 1)[0], runs, "limit {limit}");
    }
}

/// The check counts the dwords that differ between our copy and the
/// program's, the copies that had any, and the copies not compared.
fn the_check_counts_differences(gpu: &mut Gpu) {
    let copy = pass(&gpu.device, COPY).expect("our copy compiles");
    let compare = pass(&gpu.device, COMPARE).expect("the compare compiles");
    let (records, _, records_gpu) = gpu.buffer(64 * 16, false);
    let (_counts, counts, counts_gpu) = gpu.buffer(64, true);
    let (bytes, at) = ((64u32 << 10) + 64, 0x40usize);
    let len = at + bytes as usize + TAIL;
    let (_s, s, s_gpu) = gpu.buffer(len, true);
    let (_ours, _, ours_gpu) = gpu.buffer(len, false);
    let (_theirs, theirs, theirs_gpu) = gpu.buffer(len, true);
    let src = source(len, at, bytes, 8);
    put(s, &src);
    let mut copied = vec![0xCD; len];
    title(&src[at..], &mut copied[at..], 0x1000, 4096);
    let run = |gpu: &mut Gpu, limit: u32, record: u32, flips: &[usize]| {
        let mut wrong = copied.clone();
        for &f in flips {
            wrong[at + f] ^= 0x10;
        }
        put(theirs, &wrong);
        let args = CopyArgs {
            src: s_gpu + at as u64,
            dst: ours_gpu,
            records: records_gpu,
            counts: counts_gpu,
            notes: counts_gpu,
            constants: constants(0x1000, limit, record, [128, 1, 1], Notes::default()),
            groups: [128, 1, 1],
        };
        record_ours(&gpu.list, &copy, &args, &records, false);
        record_compare(
            &gpu.list,
            &compare,
            [ours_gpu, theirs_gpu + at as u64, records_gpu, counts_gpu],
            record,
        );
        gpu.run();
        let c = words(counts, count::ALL);
        [
            count::COMPARED,
            count::DWORDS,
            count::DIFFERING,
            count::COPIES_DIFFERING,
            count::NOT_COMPARED,
        ]
        .map(|i| c[i / 4])
    };
    let dwords = bytes / 4;
    // Three dwords differ (two bytes in one of them).
    assert_eq!(
        run(gpu, bytes, 0, &[0, 1, 4096, bytes as usize - 1]),
        [1, dwords, 3, 1, 0]
    );
    assert_eq!(run(gpu, bytes, 16, &[]), [2, 2 * dwords, 3, 1, 0]);
    // Ours could not copy: not compared.
    assert_eq!(run(gpu, bytes - 64, 32, &[0]), [2, 2 * dwords, 3, 1, 1]);
}

macro_rules! on_both {
    ($($name:ident: $body:ident;)*) => {
        $(
            mod $name {
                #[test]
                fn on_warp() {
                    let _one = super::ONE.lock().unwrap_or_else(|e| e.into_inner());
                    if let Some(mut gpu) = super::Gpu::new(true) {
                        super::$body(&mut gpu);
                    }
                }

                #[test]
                #[ignore = "needs a GPU: cargo test -p victory-gpu --lib bvh_copy -- --ignored"]
                fn on_the_gpu() {
                    let _one = super::ONE.lock().unwrap_or_else(|e| e.into_inner());
                    let mut gpu = super::Gpu::new(false).expect("a hardware adapter");
                    super::$body(&mut gpu);
                }
            }
        )*
    };
}

on_both! {
    copies: copies_as_the_title;
    too_long: too_long_copies_nothing;
    notes: leaves_the_programs_notes;
    predication: the_program_runs_only_where_ours_did_not;
    check: the_check_counts_differences;
}
