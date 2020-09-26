use super::{page_align_down, Frame, FrameAllocator, LockedFrameAllocator, PAGE_SIZE};
use crate::init_mutex::InitMutex;
use alloc::vec;
use bootloader::bootinfo::{MemoryRegion, MemoryRegionType};

fn set_bit(bitmask: &mut [u8], index: usize, value: bool) {
    let index_byte = index / 8;
    let index_bit = index % 8;
    let bit_mask = 1 << index_bit;

    if value {
        bitmask[index_byte] |= bit_mask;
    } else {
        bitmask[index_byte] &= !bit_mask;
    }

    assert_eq!(get_bit(bitmask, index), value);
}

fn get_bit(bitmask: &[u8], index: usize) -> bool {
    let index_byte = index / 8;
    let index_bit = index % 8;
    let bit_mask = 1 << index_bit;

    (bitmask[index_byte] & bit_mask) != 0
}

fn lowest_one_bit(byte: u8) -> Option<usize> {
    for bit in 0..8 {
        let bit_mask = 1 << bit;
        if byte & bit_mask != 0 {
            return Some(bit);
        }
    }

    None
}

struct FreeMemoryRegion {
    base: usize,
    limit: usize,
}

struct MemoryMapFilter<
    'a,
    Iter: Iterator<Item = &'a MemoryRegion>,
    CheckFn: Fn(MemoryRegionType) -> bool,
> {
    start_frame_addr: usize,
    limit_frame_addr: usize,
    iter: Iter,
    check_type: CheckFn,
}

impl<'a, Iter: Iterator<Item = &'a MemoryRegion>, CheckFn: Fn(MemoryRegionType) -> bool> Iterator
    for MemoryMapFilter<'a, Iter, CheckFn>
{
    type Item = FreeMemoryRegion;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            match self.iter.next() {
                None => return None,
                Some(region) => {
                    let base = (region.range.start_addr() as usize).max(self.start_frame_addr);
                    let limit = (region.range.end_addr() as usize).min(self.limit_frame_addr);

                    if limit > base && (self.check_type)(region.region_type) {
                        return Some(FreeMemoryRegion { base, limit });
                    }
                }
            }
        }
    }
}

fn filter_memory_map<
    'a,
    IntoIter: IntoIterator<Item = &'a MemoryRegion>,
    CheckFn: Fn(MemoryRegionType) -> bool,
>(
    start_frame: usize,
    limit_frame: usize,
    memory_map: IntoIter,
    check_type: CheckFn,
) -> MemoryMapFilter<'a, IntoIter::IntoIter, CheckFn> {
    const MINIMUM_ADDRESS: usize = 0x1_0000;

    let start_frame_addr = (start_frame * PAGE_SIZE).max(MINIMUM_ADDRESS);
    let limit_frame_addr = (limit_frame * PAGE_SIZE).max(MINIMUM_ADDRESS);

    MemoryMapFilter {
        start_frame_addr,
        limit_frame_addr,
        iter: memory_map.into_iter(),
        check_type,
    }
}

fn usable(region_type: MemoryRegionType) -> bool {
    region_type == MemoryRegionType::Usable
}

fn reclaimable(region_type: MemoryRegionType) -> bool {
    region_type == MemoryRegionType::KernelStack
        || region_type == MemoryRegionType::PageTable
        || region_type == MemoryRegionType::Bootloader
        || region_type == MemoryRegionType::BootInfo
        || region_type == MemoryRegionType::Package
}

fn usable_or_reclaimable(region_type: MemoryRegionType) -> bool {
    usable(region_type) || reclaimable(region_type)
}

fn find_available_limit_frame<'a>(
    start_frame: usize,
    limit_frame: usize,
    memory_map: impl IntoIterator<Item = &'a MemoryRegion>,
) -> usize {
    let mut available_limit_frame = start_frame;
    for region in filter_memory_map(start_frame, limit_frame, memory_map, usable_or_reclaimable) {
        let region_limit_frame = (region.limit / PAGE_SIZE).min(limit_frame);
        available_limit_frame = available_limit_frame.max(region_limit_frame);
    }
    available_limit_frame
}

pub struct PageFrameRegion {
    start_frame: usize,
    limit_frame: usize,
    free_frames: usize,
    used_frames: usize,
    bitmask: &'static mut [u8],
}

impl PageFrameRegion {
    pub fn new<'a>(
        start_frame: usize,
        limit_frame: usize,
        memory_map: impl IntoIterator<Item = &'a MemoryRegion>,
        bitmask: &'static mut [u8],
    ) -> Self {
        let mut free_frames = 0;
        bitmask.fill(0);

        for region in filter_memory_map(start_frame, limit_frame, memory_map, usable) {
            let free_span_start_frame = (region.base / PAGE_SIZE).max(start_frame) - start_frame;
            let free_span_end_frame = (region.limit / PAGE_SIZE).min(limit_frame) - start_frame;

            for free_frame in free_span_start_frame..free_span_end_frame {
                set_bit(bitmask, free_frame, true);
                free_frames += 1;
            }
        }

        Self {
            start_frame,
            limit_frame,
            free_frames,
            used_frames: 0,
            bitmask,
        }
    }

    pub fn alloc<'a>(
        start_frame: usize,
        limit_frame: usize,
        memory_map: impl IntoIterator<Item = &'a MemoryRegion> + Clone,
    ) -> Self {
        // Every page of memory for the bitmask covers 128 megabytes of physical memory. For very large memories the heap allocation in here will
        // probably not work, but it is good enough for now
        let bitmask_frames =
            find_available_limit_frame(start_frame, limit_frame, memory_map.clone()) - start_frame;
        let bitmask_bytes = (bitmask_frames + 7) / 8;

        let bitmask = vec![0; bitmask_bytes].into_boxed_slice();
        Self::new(
            start_frame,
            limit_frame,
            memory_map,
            alloc::boxed::Box::leak(bitmask),
        )
    }

    pub fn reclaim<'a>(&mut self, memory_map: impl IntoIterator<Item = &'a MemoryRegion> + Clone) {
        for region in filter_memory_map(self.start_frame, self.limit_frame, memory_map, reclaimable)
        {
            let free_span_start_frame =
                (region.base / PAGE_SIZE).max(self.start_frame) - self.start_frame;
            let free_span_end_frame =
                (region.limit / PAGE_SIZE).min(self.limit_frame) - self.start_frame;

            for free_frame in free_span_start_frame..free_span_end_frame {
                assert!(
                    get_bit(self.bitmask, free_frame) == false,
                    "Reclaiming frame that is already marked free: {:#x}",
                    free_frame
                );
                set_bit(self.bitmask, free_frame, true);
                self.free_frames += 1;
            }
        }
    }
}

impl LockedFrameAllocator for PageFrameRegion {
    fn free_frames(&self) -> usize {
        self.free_frames
    }

    fn used_frames(&self) -> usize {
        self.used_frames
    }

    fn allocate_frame(&mut self) -> Option<Frame> {
        if let Some((byte_index, byte)) = self
            .bitmask
            .iter_mut()
            .enumerate()
            .find(|(_, byte)| **byte != 0)
        {
            let bit_index = lowest_one_bit(*byte).unwrap();
            let frame_index = (byte_index * 8) + bit_index;

            // There is a possibility that the bit might be outside the range of the region because the bitmask
            // is bigger than the region. That can't happen though because we would never have set that bit to one
            debug_assert!(frame_index < self.limit_frame);

            set_bit(self.bitmask, frame_index, false);
            self.free_frames -= 1;
            self.used_frames += 1;

            Some(Frame::from_index(frame_index + self.start_frame))
        } else {
            None
        }
    }

    fn deallocate_frame(&mut self, frame: Frame) {
        assert!(self.contains_frame(frame), "Frame is not from this region");

        let frame_index = frame.index() - self.start_frame;
        set_bit(self.bitmask, frame_index, true);
        self.free_frames += 1;
        self.used_frames -= 1;
    }

    fn contains_frame(&self, frame: Frame) -> bool {
        frame.index() >= self.start_frame && frame.index() < self.limit_frame
    }
}

// Traditionally the low region is "the region addressable by the ISA DMA controller".
// I probably don't care about the ISA DMA controller, but I need to have some limit of
// how much memory I want to statically initialize before paging is up and running, so 16MiB
// seems like a good amount
const LOW_REGION_BASE: usize = 64 * 1024; // Don't use the first 64KiB - it is useful to have it free
const UNUSED_LOW_FRAMES: usize = LOW_REGION_BASE / PAGE_SIZE;
const LOW_REGION_SIZE_LIMIT: usize = 16 * 1024 * 1024;
const LOW_REGION_FRAMES: usize = LOW_REGION_SIZE_LIMIT / PAGE_SIZE;

pub static LOW_REGION: InitMutex<PageFrameRegion> = InitMutex::new();

// The normal region is the region we prefer for kernel allocations - it is useful because it is
// permanently mapped in kernel address space, so we don't have to worry about mapping pages.
const NORMAL_REGION_SIZE_LIMIT: usize = 4 * 1024 * 1024 * 1024;
const NORMAL_REGION_FRAMES: usize = NORMAL_REGION_SIZE_LIMIT / PAGE_SIZE;

pub static NORMAL_REGION: InitMutex<PageFrameRegion> = InitMutex::new();

// The high region is everything else
const HIGH_REGION_SIZE_LIMIT: usize = page_align_down(core::usize::MAX);
const HIGH_REGION_FRAMES: usize = HIGH_REGION_SIZE_LIMIT / PAGE_SIZE;

pub static HIGH_REGION: InitMutex<PageFrameRegion> = InitMutex::new();

pub fn early_init<'a, T: IntoIterator<Item = &'a MemoryRegion>>(memory_map: T) {
    fn make_early_memory_map<'a, T: IntoIterator<Item = &'a MemoryRegion>>(
        memory_map: T,
    ) -> PageFrameRegion {
        const LOW_REGION_BITMASK_BYTES: usize = (LOW_REGION_FRAMES + 7) / 8;
        static mut LOW_REGION_BITMASK: [u8; LOW_REGION_BITMASK_BYTES] =
            [0; LOW_REGION_BITMASK_BYTES];

        // We need an unsafe here because we're using a mutable static, but it is safe because the init mutex
        // guarantees this function will only be called once
        PageFrameRegion::new(UNUSED_LOW_FRAMES, LOW_REGION_FRAMES, memory_map, unsafe {
            &mut LOW_REGION_BITMASK
        })
    }

    LOW_REGION.init(make_early_memory_map(memory_map));
}

pub fn init_post_paging<'a>(memory_map: impl IntoIterator<Item = &'a MemoryRegion> + Clone) {
    NORMAL_REGION.init(PageFrameRegion::alloc(
        LOW_REGION_FRAMES,
        NORMAL_REGION_FRAMES,
        memory_map.clone(),
    ));
    HIGH_REGION.init(PageFrameRegion::alloc(
        NORMAL_REGION_FRAMES,
        HIGH_REGION_FRAMES,
        memory_map,
    ));
}

pub fn init_reclaim<'a>(memory_map: impl IntoIterator<Item = &'a MemoryRegion> + Clone) {
    LOW_REGION.lock().reclaim(memory_map.clone());
    NORMAL_REGION.lock().reclaim(memory_map.clone());
    HIGH_REGION.lock().reclaim(memory_map);
}

impl<T: LockedFrameAllocator> FrameAllocator for InitMutex<T> {
    fn free_frames(&self) -> usize {
        self.try_lock()
            .map(|guard| guard.free_frames())
            .unwrap_or(0)
    }

    fn used_frames(&self) -> usize {
        self.try_lock()
            .map(|guard| guard.used_frames())
            .unwrap_or(0)
    }

    fn allocate_frame(&self) -> Option<Frame> {
        self.try_lock().and_then(|mut guard| guard.allocate_frame())
    }

    fn deallocate_frame(&self, frame: Frame) {
        self.lock().deallocate_frame(frame)
    }

    fn contains_frame(&self, frame: Frame) -> bool {
        self.try_lock()
            .map(|guard| guard.contains_frame(frame))
            .unwrap_or(false)
    }
}
