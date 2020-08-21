use super::frame::Frame;
use super::page::{PageSize, Size4KiB};
use crate::addr::PhysicalAddress;
use bit_field::BitField;
use bootloader::{bootinfo::MemoryRegionType, BootInfo};
use core::ops::Range;

pub unsafe trait FrameAllocator<S: PageSize> {
    fn allocate_frame(&mut self) -> Option<Frame<S>>;
}

pub trait FrameDeallocator<S: PageSize> {
    unsafe fn deallocate_frame(&mut self, frame: Frame<S>);
}

// The goal of the early frame allocator is to be able to represent enough physical memory
// to get us far enough into the boot that we can allocate the real frame allocator.
// The real frame allocator is likely to involve trees and such, and so will need a working heap
// to operate. To keep things simple, we're just going to use a bitmask large enough for the
// first 32MiB of memory
const EARLY_FRAME_MAX_PHYS_MEMORY: usize = 32 * 1024 * 1024;
const EARLY_FRAME_MAX_PAGES: usize = EARLY_FRAME_MAX_PHYS_MEMORY / Size4KiB::SIZE as usize;

const EARLY_FRAME_MAX_WORDS: usize = EARLY_FRAME_MAX_PAGES / 64;

#[derive(Debug)]
pub struct EarlyFrameAllocator {
    bitmask: [u64; EARLY_FRAME_MAX_WORDS],
    available_frames: usize,
}

impl EarlyFrameAllocator {
    pub unsafe fn from_boot_info(boot_info: &'static BootInfo) -> Self {
        let mut ret = Self {
            bitmask: [0; EARLY_FRAME_MAX_WORDS],
            available_frames: 0,
        };

        for range in boot_info.memory_map.iter() {
            // For now, we're only going to use memory that is marked as usable. Later on,
            // when we have our own copy of the boot info and we know we don't need the bootloader
            // any more we will recover some of that memory.
            if range.region_type == MemoryRegionType::Usable {
                let start_frame: Result<Frame, _> =
                    PhysicalAddress::try_new(range.range.start_addr())
                        .map_err(|_| ())
                        .and_then(|a| Frame::from_start_address(a));
                let end_frame: Result<Frame, _> = PhysicalAddress::try_new(range.range.end_addr())
                    .map_err(|_| ())
                    .and_then(|a| Frame::from_start_address(a));

                if let (Ok(start_frame), Ok(end_frame)) = (start_frame, end_frame) {
                    ret.mark_range_as_usable(start_frame..end_frame);
                }
            }
        }

        ret
    }

    pub unsafe fn mark_range_as_usable(&mut self, range: Range<Frame>) {
        for frame in range {
            if let Some((word, bit)) = Self::split_frame(frame) {
                if !self.bitmask[word].get_bit(bit) {
                    self.bitmask[word].set_bit(bit, true);
                    self.available_frames += 1;
                }
            }
        }
    }

    fn split_frame(frame: Frame) -> Option<(usize, usize)> {
        let start_addr = frame.start_address().as_u64() / Size4KiB::SIZE;
        let word = start_addr as usize / 64;
        if word < EARLY_FRAME_MAX_WORDS {
            Some((word as usize, start_addr as usize % 64))
        } else {
            None
        }
    }

    unsafe fn build_frame(word: usize, bit: usize) -> Frame {
        let start_addr = ((word * 64) + bit) as u64 * Size4KiB::SIZE;
        let start_addr = PhysicalAddress::new(start_addr);
        Frame::from_start_address_unchecked(start_addr)
    }
}

unsafe impl FrameAllocator<Size4KiB> for EarlyFrameAllocator {
    fn allocate_frame(&mut self) -> Option<Frame<Size4KiB>> {
        if self.available_frames > 0 {
            for (word_idx, word) in self.bitmask.iter_mut().enumerate() {
                if *word != 0 {
                    for bit in 0..64 {
                        if word.get_bit(bit) {
                            word.set_bit(bit, false);
                            self.available_frames -= 1;
                            return Some(unsafe { Self::build_frame(word_idx, bit) });
                        }
                    }

                    panic!("Failed to find available bit in non-zero word");
                }
            }

            panic!("Failed to find available page when counter says there should be one");
        }

        None
    }
}

impl FrameDeallocator<Size4KiB> for EarlyFrameAllocator {
    unsafe fn deallocate_frame(&mut self, frame: Frame<Size4KiB>) {
        let (word_idx, bit) =
            Self::split_frame(frame).expect("The frame is not in the range of this allocator");

        assert!(
            !self.bitmask[word_idx].get_bit(bit),
            "The frame being deallocated is not marked as in use"
        );

        self.bitmask[word_idx].set_bit(bit, true);
        self.available_frames += 1;
    }
}
