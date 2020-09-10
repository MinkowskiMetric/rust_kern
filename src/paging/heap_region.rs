use super::{
    lock_page_table, p1_index, p2_index, p3_index, p4_index, ActivePageTable,
    MappedMutPteReference, MemoryError, PageFlags, PageTable, PageTableEntry, Result, L1, L2,
    PAGE_SIZE,
};
use crate::physmem::{allocate_frame, Frame};
use alloc::vec::Vec;
use bitflags::bitflags;
use core::mem::MaybeUninit;
use core::ops::{Deref, DerefMut};
use spin::Mutex;

fn align_down(addr: usize, align: usize) -> usize {
    if align.is_power_of_two() {
        addr & !(align - 1)
    } else if align == 0 {
        addr
    } else {
        panic!("`align` must be a power of 2");
    }
}

/// Align upwards. Returns the smallest x with alignment `align`
/// so that x >= addr. The alignment must be a power of 2.
fn align_up(addr: usize, align: usize) -> usize {
    align_down(addr + align - 1, align)
}

bitflags! {
    pub struct RegionFlags: u64 {
        const NON_PAGES = 1 << 0;
    }
}

struct RegionManager {
    base: u64,
    limit: u64,
}

// There are 14 free bits in an allocated page entry. We need to use some of those for flags.
// So, how big can a region be. Lets say we save two bits for flags. That leaves us 12 bits.
// If we quantize the size of the regions at 64KB, then that gives us a maximum region size of 256MB
// which ought to be big enough for anyone. It isn't enough to cover the whole heap region though,
// so the search needs to take that into account

const REGION_ALIGNMENT_PAGES: usize = 16;
const REGION_CHUNK_SIZE: usize = REGION_ALIGNMENT_PAGES * PAGE_SIZE as usize;
const REGION_CHUNK_COUNT_BITS: usize = 12;
const REGION_CHUNK_COUNT_SHIFT: usize = 16;
const MAX_REGION_CHUNKS: usize = 1 << REGION_CHUNK_COUNT_BITS;
const MAXIMUM_REGION_SIZE: usize = MAX_REGION_CHUNKS * REGION_CHUNK_SIZE;

fn is_region_chunk_aligned(addr: u64) -> bool {
    (addr as usize & (REGION_CHUNK_SIZE - 1)) == 0
}

fn make_region_flags_page_flags(region_flags: RegionFlags) -> PageFlags {
    // Currently we don't use the region flags, but to make sure everything is fine, we set them
    PageFlags::BIT_62 | PageFlags::BIT_61
}

fn make_region_chunk_count_flags_flags(
    region_chunk_count: u64,
    region_flags: RegionFlags,
) -> PageFlags {
    let write_chunk_count = if region_chunk_count == MAX_REGION_CHUNKS as u64 {
        0
    } else {
        assert!(region_chunk_count < MAX_REGION_CHUNKS as u64);
        region_chunk_count
    };

    let part_1 = PageFlags::from_bits_truncate((write_chunk_count & 0b000000000111) << 9);
    let part_2 = PageFlags::from_bits_truncate((write_chunk_count & 0b111111111000) << 49);
    part_1 | part_2 | make_region_flags_page_flags(region_flags)
}

fn make_region_header_page_entry(
    addr: u64,
    flags: PageFlags,
    region_chunk_count: u64,
    region_flags: RegionFlags,
) -> PageTableEntry {
    const CHUNK_COUNT_FLAGS: PageFlags =
        PageFlags::from_bits_truncate((0b111 << 9) | (0b111111111000 << 49));

    PageTableEntry::from_frame_and_flags(
        Frame::containing_address(addr),
        (flags & !CHUNK_COUNT_FLAGS)
            | make_region_chunk_count_flags_flags(region_chunk_count, region_flags),
    )
}

fn get_chunks_from_pte(pte: PageTableEntry) -> usize {
    let flags = pte.flags();
    let part_1 = (flags.bits() >> 9) & 0b000000000111;
    let part_2 = (flags.bits() >> 49) & 0b111111111000;
    if part_1 == 0 && part_2 == 0 {
        MAX_REGION_CHUNKS
    } else {
        (part_1 | part_2) as usize
    }
}

struct RegionHeader {
    pte_1: MappedMutPteReference<L1>,
    pte_2: MappedMutPteReference<L1>,
}

impl RegionHeader {
    fn create(page_table: &mut ActivePageTable, start_addr: u64) -> Result<Self> {
        Ok(Self {
            pte_1: page_table.create_pte_mut_for_address(start_addr)?,
            pte_2: page_table.create_pte_mut_for_address(start_addr + PAGE_SIZE)?,
        })
    }

    fn this_region_size_in_chunks(&self) -> usize {
        get_chunks_from_pte(*self.pte_1)
    }

    fn prev_region_size_in_chunks(&self) -> usize {
        get_chunks_from_pte(*self.pte_2)
    }

    fn set_this_region_size_in_chunks(&mut self, size: usize, flags: RegionFlags) {
        *self.pte_1 = make_region_header_page_entry(
            (*self.pte_1).frame().physical_address(),
            (*self.pte_1).flags(),
            size as u64,
            flags,
        )
    }

    fn set_prev_region_size_in_chunks(&mut self, size: usize, flags: RegionFlags) {
        *self.pte_2 = make_region_header_page_entry(
            (*self.pte_1).frame().physical_address(),
            (*self.pte_1).flags(),
            size as u64,
            flags,
        )
    }
}

impl RegionManager {
    pub fn new(base: u64, limit: u64) -> Result<Self> {
        // Base needs to be an even page index to ensure that the first and second pages
        // as in the same L1 table
        let base = align_up(base as usize, PAGE_SIZE as usize * 2) as u64;
        let limit = align_down(limit as usize, PAGE_SIZE as usize) as u64;
        assert!(limit > base);

        let size_in_region_chunks = (limit - base) / REGION_CHUNK_SIZE as u64;
        assert!(size_in_region_chunks > 0);

        // We use 2 page table entries to record data about the chunk. We write the chunk size in page 0
        // the previous chunk size in page 1
        assert!(REGION_ALIGNMENT_PAGES >= 2);

        let mut page_table = unsafe { lock_page_table() }?;

        // Create free chunks starting at the base address
        let mut free_region_start = base;
        let mut remaining_region_chunks = size_in_region_chunks;
        let mut last_region_chunks = 0;
        while remaining_region_chunks > 0 {
            let this_region_chunks = remaining_region_chunks.min(MAX_REGION_CHUNKS as u64);

            let mut region_header = RegionHeader::create(&mut page_table, free_region_start)?;
            region_header
                .set_this_region_size_in_chunks(this_region_chunks as usize, RegionFlags::empty());
            region_header
                .set_prev_region_size_in_chunks(last_region_chunks as usize, RegionFlags::empty());

            last_region_chunks = this_region_chunks;
            remaining_region_chunks -= this_region_chunks;
            free_region_start += (this_region_chunks * REGION_CHUNK_SIZE as u64);
        }

        assert!(free_region_start == limit);

        Ok(RegionManager { base, limit })
    }

    pub fn allocate_region(&mut self, required_pages: usize, flags: RegionFlags) -> Result<Region> {
        let required_chunks =
            align_up(required_pages as usize, REGION_ALIGNMENT_PAGES) / REGION_ALIGNMENT_PAGES;

        unsafe { lock_page_table() }.and_then(|mut page_table| {
            let mut this_region_start = self.base;
            while this_region_start < self.limit {
                let mut pte = page_table
                    .get_pte_for_address(this_region_start)
                    .expect("Failed to map region page table");
                let this_region_chunks = get_chunks_from_pte(*pte);

                use crate::println;

                if !pte.flags().contains(PageFlags::PRESENT)
                    && this_region_chunks >= required_chunks
                {
                    if this_region_chunks > required_chunks {
                        self.split_region(
                            &mut page_table,
                            this_region_start,
                            this_region_chunks,
                            required_chunks,
                        )?;
                    }

                    return self
                        .map_region(&mut page_table, this_region_start, required_chunks)
                        .map(|_| Region {
                            start_va: this_region_start,
                            limit_va: this_region_start
                                + ((required_chunks * REGION_CHUNK_SIZE) as u64),
                        });
                }

                println!(
                    "Found {} chunk region at {:#x} with flags {:?}",
                    this_region_chunks,
                    this_region_start,
                    pte.flags()
                );
                assert!(this_region_chunks != 0);
                this_region_start += (this_region_chunks * REGION_CHUNK_SIZE) as u64;
            }

            Err(MemoryError::NoRegionAddressSpaceAvailable)
        })
    }

    pub fn release_region(&mut self, start_va: u64, limit_va: u64) {
        todo!()
    }

    fn split_region(
        &mut self,
        page_table: &mut ActivePageTable,
        this_region_start: u64,
        original_chunks: usize,
        new_chunks: usize,
    ) -> Result<()> {
        let new_region_start = this_region_start + (new_chunks * REGION_CHUNK_SIZE) as u64;
        let next_region_start = this_region_start + (original_chunks * REGION_CHUNK_SIZE) as u64;

        // Do all the operations that can fail up front
        let mut this_region_header = RegionHeader::create(page_table, this_region_start)?;
        let mut new_region_header = RegionHeader::create(page_table, new_region_start)?;
        let mut next_region_header = if next_region_start < self.limit {
            Some(RegionHeader::create(page_table, next_region_start)?)
        } else {
            None
        };

        // At this point we cannot fail
        this_region_header.set_this_region_size_in_chunks(new_chunks, RegionFlags::empty());

        new_region_header
            .set_this_region_size_in_chunks(original_chunks - new_chunks, RegionFlags::empty());
        new_region_header.set_prev_region_size_in_chunks(new_chunks, RegionFlags::empty());

        if let Some(mut next_region) = next_region_header {
            next_region
                .set_prev_region_size_in_chunks(original_chunks - new_chunks, RegionFlags::empty());
        }

        Ok(())
    }

    fn map_region(
        &mut self,
        page_table: &mut ActivePageTable,
        region_start: u64,
        region_chunks: usize,
    ) -> Result<()> {
        for chunk_index in 0..region_chunks {
            let map_chunk_result: Result<()> = {
                let chunk_start = region_start + (chunk_index * REGION_CHUNK_SIZE) as u64;

                for page_idx in 0..REGION_ALIGNMENT_PAGES {
                    let page_start = chunk_start + (page_idx as u64 * PAGE_SIZE);

                    let mut pte = page_table.create_pte_mut_for_address(page_start)?;

                    // The PTE should not be marked as present or writeable
                    let original_flags = pte.flags();
                    assert!(!original_flags.intersects(PageFlags::PRESENT | PageFlags::WRITABLE));

                    // Allocate a physical page and assign it to the page.
                    let frame = crate::physmem::allocate_frame().ok_or(MemoryError::OutOfMemory)?;
                    *pte = PageTableEntry::from_frame_and_flags(
                        frame,
                        original_flags
                            | PageFlags::PRESENT
                            | PageFlags::WRITABLE
                            | PageFlags::NO_EXECUTE,
                    );
                }

                Ok(())
            };

            if let Err(e) = map_chunk_result {
                // Technically, it is possible that not all of this region is mapped. We might
                // have finished part way through mapping the last chunk, but unmap_region can
                // tolerate that.
                self.unmap_region(page_table, region_start, chunk_index + 1);
                return Err(e);
            }
        }

        Ok(())
    }

    fn unmap_region(
        &mut self,
        page_table: &mut ActivePageTable,
        region_start: u64,
        region_chunks: usize,
    ) {
        todo!()
    }
}

static REGION_MANAGER: Mutex<Option<RegionManager>> = Mutex::new(None);

#[repr(transparent)]
struct RegionManagerLock<'a> {
    guard: spin::MutexGuard<'a, Option<RegionManager>>,
}

impl<'a> Deref for RegionManagerLock<'a> {
    type Target = RegionManager;
    fn deref(&self) -> &Self::Target {
        self.guard.as_ref().expect("Region manager not initialized")
    }
}

impl<'a> DerefMut for RegionManagerLock<'a> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.guard.as_mut().expect("Region manager not initialized")
    }
}

fn lock_region_manager<'a>() -> RegionManagerLock<'a> {
    RegionManagerLock {
        guard: REGION_MANAGER.lock(),
    }
}

pub struct Region {
    start_va: u64,
    limit_va: u64,
}

impl Region {
    pub fn as_ptr<T>(&self) -> *const T {
        self.as_ptr_offset(0)
    }

    pub fn as_ptr_offset<T>(&self, offset: usize) -> *const T {
        (self.start_va + (offset as u64)) as *const T
    }
    pub fn as_mut_ptr<T>(&mut self) -> *mut T {
        self.as_mut_ptr_offset(0)
    }

    pub fn as_mut_ptr_offset<T>(&mut self, offset: usize) -> *mut T {
        (self.start_va + (offset as u64)) as *mut T
    }

    pub fn start(&self) -> u64 {
        self.start_va
    }

    pub fn limit(&self) -> u64 {
        self.limit_va
    }
}

impl Drop for Region {
    fn drop(&mut self) {
        lock_region_manager().release_region(self.start_va, self.limit_va);
    }
}

pub unsafe fn init(base: u64, limit: u64) -> Result<()> {
    *REGION_MANAGER.lock() = Some(RegionManager::new(base, limit)?);
    Ok(())
}

pub fn allocate_region(pages: usize, flags: RegionFlags) -> Result<Region> {
    lock_region_manager().allocate_region(pages, flags)
}
