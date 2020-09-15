use super::page_entry::{self, PresentPageFlags, RawPresentPte};
use super::{
    lock_page_table, p1_index, p2_index, p3_index, p4_index, ActivePageTable,
    MappedMutPteReference, MemoryError, PageTable, Result, L1, L2, PAGE_SIZE,
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
const MAX_REGION_CHUNKS: usize = RawPresentPte::MAX_COUNTER_VALUE as usize;
const MAXIMUM_REGION_SIZE: usize = MAX_REGION_CHUNKS * REGION_CHUNK_SIZE;

/*struct RegionHeader {
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

    fn is_free_region(&self) -> bool {
        !self.pte_1.is_present()
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
}*/

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

        /*let mut page_table = unsafe { lock_page_table() }?;

        // Create free chunks starting at the base address
        Self::create_free_region_chain(&mut page_table, base, size_in_region_chunks)?;*/

        Ok(RegionManager { base, limit })
    }

    pub fn allocate_region(&mut self, required_pages: usize, flags: RegionFlags) -> Result<Region> {
        /*let required_chunks =
            align_up(required_pages as usize, REGION_ALIGNMENT_PAGES) / REGION_ALIGNMENT_PAGES;

        unsafe { lock_page_table() }.and_then(|mut page_table| {
            let mut this_region_start = self.base;
            while this_region_start < self.limit {
                let mut pte = page_table
                    .get_pte_for_address(this_region_start)
                    .expect("Failed to map region page table");
                let this_region_chunks = get_chunks_from_pte(*pte);

                if !pte.is_present() && this_region_chunks >= required_chunks {
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

                assert!(this_region_chunks != 0);
                this_region_start += (this_region_chunks * REGION_CHUNK_SIZE) as u64;
            }

            Err(MemoryError::NoRegionAddressSpaceAvailable)
        })*/
        todo!()
    }

    pub fn release_region(&mut self, start_va: u64, limit_va: u64) {
        /*assert_eq!(
            start_va & !(REGION_CHUNK_SIZE as u64 - 1),
            start_va,
            "Invalid start address"
        );
        assert!(start_va >= self.base, "Invalid start address");
        assert_eq!(
            limit_va & !(REGION_CHUNK_SIZE as u64 - 1),
            limit_va,
            "Invalid limit address"
        );
        assert!(
            limit_va >= start_va && limit_va <= self.limit,
            "Invalid limit address"
        );

        let size_in_chunks = (limit_va - start_va) / REGION_CHUNK_SIZE as u64;
        unsafe { lock_page_table() }
            .and_then(|mut page_table| {
                let prev_region_size = {
                    let mut region_header = RegionHeader::create(&mut page_table, start_va)?;
                    assert_eq!(
                        region_header.this_region_size_in_chunks() as u64,
                        size_in_chunks,
                        "Invalid region size"
                    );
                    assert!(!region_header.is_free_region(), "Double freeing region");

                    // We ignore the previous region size for the first region because the special
                    // encoding will catch us out
                    if start_va > self.base {
                        region_header.prev_region_size_in_chunks()
                    } else {
                        0
                    }
                };
                assert!(
                    (start_va - self.base) >= (prev_region_size * REGION_CHUNK_SIZE) as u64,
                    "Invalid prev region size"
                );

                // Unmap any pages in the region
                self.unmap_region(&mut page_table, start_va, size_in_chunks as usize);

                // We need to count forward how many free chunks there are after this one
                let mut free_chunks = size_in_chunks;
                let mut free_chunks_end = limit_va;
                let mut last_chunk_size = size_in_chunks;
                while free_chunks_end < self.limit {
                    let mut following_region_header =
                        RegionHeader::create(&mut page_table, free_chunks_end)?;
                    assert_eq!(
                        following_region_header.prev_region_size_in_chunks() as u64,
                        last_chunk_size,
                        "Invalid prev region size"
                    );

                    if !following_region_header.is_free_region() {
                        break;
                    }

                    // We're going to include this region into the free region block we're creating
                    last_chunk_size = following_region_header.this_region_size_in_chunks() as u64;
                    free_chunks += last_chunk_size;
                    free_chunks_end += (last_chunk_size * REGION_CHUNK_SIZE as u64);
                    assert!(free_chunks_end <= self.limit, "Invalid region size");
                }

                let free_chunks_start = start_va
                    - if prev_region_size > 0 {
                        let mut prev_region_header = RegionHeader::create(
                            &mut page_table,
                            start_va - (prev_region_size * REGION_CHUNK_SIZE) as u64,
                        )?;
                        assert_eq!(
                            prev_region_header.this_region_size_in_chunks(),
                            prev_region_size,
                            "Invalid prev region size"
                        );

                        // If the previous region is free then merge the two
                        if prev_region_header.is_free_region() {
                            free_chunks += prev_region_size as u64;
                            (prev_region_size * REGION_CHUNK_SIZE) as u64
                        } else {
                            0
                        }
                    } else {
                        0
                    };

                Self::create_free_region_chain(&mut page_table, free_chunks_start, free_chunks)
            })
            .expect("what if this fails!?");*/
        todo!()
    }

    fn split_region(
        &mut self,
        page_table: &mut ActivePageTable,
        this_region_start: u64,
        original_chunks: usize,
        new_chunks: usize,
    ) -> Result<()> {
        /*let new_region_start = this_region_start + (new_chunks * REGION_CHUNK_SIZE) as u64;
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

        Ok(())*/
        todo!()
    }

    fn create_free_region_chain(
        page_table: &mut ActivePageTable,
        start: u64,
        chunks: u64,
    ) -> Result<()> {
        /*let mut free_region_start = start;
        let mut remaining_region_chunks = chunks;
        let mut last_region_chunks = 0;
        while remaining_region_chunks > 0 {
            let this_region_chunks = remaining_region_chunks.min(MAX_REGION_CHUNKS as u64);

            let mut region_header = RegionHeader::create(page_table, free_region_start)?;
            region_header
                .set_this_region_size_in_chunks(this_region_chunks as usize, RegionFlags::empty());
            region_header
                .set_prev_region_size_in_chunks(last_region_chunks as usize, RegionFlags::empty());

            last_region_chunks = this_region_chunks;
            remaining_region_chunks -= this_region_chunks;
            free_region_start += (this_region_chunks * REGION_CHUNK_SIZE as u64);
        }

        Ok(())*/
        todo!()
    }

    fn map_region(
        &mut self,
        page_table: &mut ActivePageTable,
        region_start: u64,
        region_chunks: usize,
    ) -> Result<()> {
        /*for chunk_index in 0..region_chunks {
            let map_chunk_result: Result<()> = try {
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
            };

            if let Err(e) = map_chunk_result {
                // Technically, it is possible that not all of this region is mapped. We might
                // have finished part way through mapping the last chunk, but unmap_region can
                // tolerate that.
                self.unmap_region(page_table, region_start, chunk_index + 1);
                return Err(e);
            }
        }

        Ok(())*/
        todo!()
    }

    fn unmap_region(
        &mut self,
        page_table: &mut ActivePageTable,
        region_start: u64,
        region_chunks: usize,
    ) {
        /*let res: Result<()> = try {
            for page_idx in 0..(region_chunks * REGION_ALIGNMENT_PAGES) {
                let page_start = region_start + (page_idx as u64 * PAGE_SIZE);
                let mut pte = page_table.create_pte_mut_for_address(page_start)?;

                let original_flags = pte.flags();
                assert!(original_flags.contains(PageFlags::PRESENT));

                let frame = pte.frame();

                // Clear the present, writeable and no execute flags
                let new_flags = original_flags
                    & !(PageFlags::PRESENT | PageFlags::WRITABLE | PageFlags::NO_EXECUTE);

                *pte =
                    PageTableEntry::from_frame_and_flags(Frame::containing_address(0), new_flags);
                unsafe {
                    x86::tlb::flush(page_start as usize);
                }

                crate::physmem::deallocate_frame(frame);
            }
        };

        res.expect("What do we do with an error here?")*/
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
