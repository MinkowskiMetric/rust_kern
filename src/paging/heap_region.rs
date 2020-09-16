use super::page_entry::{
    InvalidPteError, NotPresentPageType, PresentPageFlags, RawNotPresentPte, RawPresentPte,
};
use super::{
    lock_page_table, p2_index, p3_index, p4_index, ActivePageTable, Frame, MapperFlushAll,
    MemoryError, Result, PAGE_SIZE,
};
use crate::init_mutex::InitMutex;
use crate::physmem;
use core::convert::{TryFrom, TryInto};
use core::fmt;

fn align_down(addr: usize, align: usize) -> usize {
    if align.is_power_of_two() {
        addr & !(align - 1)
    } else if align == 0 {
        addr
    } else {
        panic!("`align` must be a power of 2");
    }
}

fn align_up(addr: usize, align: usize) -> usize {
    align_down(addr + align - 1, align)
}

const REGION_CHUNK_PAGES: usize = 16;
const REGION_CHUNK_SIZE_IN_BYTES: usize = REGION_CHUNK_PAGES * (PAGE_SIZE as usize);
const MAXIMUM_REGION_SIZE_IN_CHUNKS: usize = RawPresentPte::MAX_COUNTER_VALUE as usize;

#[repr(transparent)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct RegionHeaderBasePte(u16);

impl RegionHeaderBasePte {
    pub fn new(region_size_in_chunks: usize) -> Self {
        let safe_counter = if region_size_in_chunks > 0
            && region_size_in_chunks <= MAXIMUM_REGION_SIZE_IN_CHUNKS
        {
            (region_size_in_chunks - 1) as u16
        } else {
            panic!("Invalid region size {}", region_size_in_chunks)
        };

        Self(safe_counter)
    }

    pub fn from_counter(counter: u16) -> Self {
        assert!(counter < RawPresentPte::MAX_COUNTER_VALUE);
        Self(counter)
    }

    pub fn size_in_chunks(&self) -> usize {
        (self.0 + 1).into()
    }

    pub fn counter(&self) -> u16 {
        self.0
    }
}

impl fmt::Debug for RegionHeaderBasePte {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_fmt(format_args!(
            "RegionHeaderBasePte({})",
            self.size_in_chunks()
        ))
    }
}

pub struct PresentRegionHeaderPte(Frame, PresentPageFlags, RegionHeaderBasePte);

impl PresentRegionHeaderPte {
    pub fn new(frame: Frame, flags: PresentPageFlags, region_size_in_chunks: usize) -> Self {
        Self(
            frame,
            flags | PresentPageFlags::REGION_HEADER,
            RegionHeaderBasePte::new(region_size_in_chunks),
        )
    }

    pub fn frame(&self) -> Frame {
        self.0
    }

    pub fn flags(&self) -> PresentPageFlags {
        self.1
    }

    pub fn size_in_chunks(&self) -> usize {
        self.2.size_in_chunks()
    }

    fn counter(&self) -> u16 {
        self.2.counter()
    }
}

impl fmt::Debug for PresentRegionHeaderPte {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_fmt(format_args!(
            "PresentRegionHeaderFirstPte({:?}, {:?}, {})",
            self.frame(),
            self.flags(),
            self.size_in_chunks()
        ))
    }
}

impl From<PresentRegionHeaderPte> for RawPresentPte {
    fn from(region_header: PresentRegionHeaderPte) -> Self {
        RawPresentPte::from_frame_flags_and_counter(
            region_header.frame(),
            region_header.flags(),
            region_header.counter(),
        )
    }
}

impl TryFrom<RawPresentPte> for PresentRegionHeaderPte {
    type Error = InvalidPteError;
    fn try_from(rpte: RawPresentPte) -> core::result::Result<Self, Self::Error> {
        if rpte.flags().contains(PresentPageFlags::REGION_HEADER) {
            Ok(Self(
                rpte.frame(),
                rpte.flags(),
                RegionHeaderBasePte::from_counter(rpte.counter()),
            ))
        } else {
            Err(InvalidPteError(rpte.into()))
        }
    }
}

pub struct NotPresentRegionHeaderPte(RegionHeaderBasePte);

impl NotPresentRegionHeaderPte {
    pub fn new(region_size_in_chunks: usize) -> Self {
        Self(RegionHeaderBasePte::new(region_size_in_chunks))
    }

    pub fn size_in_chunks(&self) -> usize {
        self.0.size_in_chunks()
    }

    fn counter(&self) -> u16 {
        self.0.counter()
    }
}

impl fmt::Debug for NotPresentRegionHeaderPte {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_fmt(format_args!(
            "NotPresentRegionHeaderPte({})",
            self.size_in_chunks()
        ))
    }
}

impl From<NotPresentRegionHeaderPte> for RawNotPresentPte {
    fn from(region_header: NotPresentRegionHeaderPte) -> Self {
        RawNotPresentPte::from_type_and_counter(
            NotPresentPageType::RegionHeader,
            region_header.counter(),
        )
    }
}

impl TryFrom<RawNotPresentPte> for NotPresentRegionHeaderPte {
    type Error = InvalidPteError;
    fn try_from(rpte: RawNotPresentPte) -> core::result::Result<Self, Self::Error> {
        if rpte.page_type() == NotPresentPageType::RegionHeader {
            Ok(Self(RegionHeaderBasePte::from_counter(rpte.counter())))
        } else {
            Err(InvalidPteError(rpte.into()))
        }
    }
}

struct RegionManager {
    base_va: usize,
    limit_va: usize,
}

#[derive(Clone, Copy)]
struct RegionInfo {
    start_va: usize,
    limit_va: usize,
}

impl RegionInfo {
    pub fn size(&self) -> usize {
        self.limit_va - self.start_va
    }
}

static REGION_MANAGER: InitMutex<RegionManager> = InitMutex::new();

impl RegionManager {
    pub unsafe fn new(base_va: usize, limit_va: usize) -> Result<Self> {
        lock_page_table().and_then(|mut page_table| {
            // Write out empty region headers through the whole space
            Self::write_empty_regions(&mut page_table, base_va, limit_va, None)?;

            Ok(Self { base_va, limit_va })
        })
    }

    pub fn allocate_region(&mut self, pages: usize) -> Result<Region> {
        let pages = align_up(pages, REGION_CHUNK_PAGES);
        let required_size = pages * (PAGE_SIZE as usize);

        unsafe { lock_page_table() }.and_then(|mut page_table| {
            let mut pos = self.base_va;
            while pos < self.limit_va {
                if let Some(mut region_info) =
                    Self::find_empty_region(&mut page_table, pos, self.limit_va)?
                {
                    if region_info.size() >= required_size {
                        if region_info.size() > required_size {
                            let next_allocated_chunk = Self::find_allocated_region(
                                &mut page_table,
                                region_info.limit_va,
                                self.limit_va,
                            )?;
                            let start_of_free_regions = region_info.start_va + required_size;
                            let end_of_free_regions = match &next_allocated_chunk {
                                Some(region_info) => region_info.start_va,
                                _ => self.limit_va,
                            };

                            // Clear the region out. This is a mutating change so we can't fail after this
                            let last_empty_chunk_size = Self::write_empty_regions(
                                &mut page_table,
                                start_of_free_regions,
                                end_of_free_regions,
                                Some(pages / REGION_CHUNK_PAGES),
                            )?
                            .unwrap();

                            // Change the size of the current region
                            Self::update_this_chunk_size(
                                &mut page_table,
                                region_info.start_va,
                                pages / REGION_CHUNK_PAGES,
                            )
                            .expect("Cannot fail after modifying regions");

                            // Fix up the back pointer in the region
                            if let Some(next_allocated_chunk) = next_allocated_chunk {
                                Self::update_previous_chunk_size(
                                    &mut page_table,
                                    next_allocated_chunk.start_va,
                                    last_empty_chunk_size,
                                )
                                .expect("Cannot fail after modifying regions");
                            }

                            region_info = RegionInfo {
                                start_va: region_info.start_va,
                                limit_va: region_info.start_va + required_size,
                            };
                        }

                        return self
                            .map_region(&mut page_table, &region_info)
                            .map(|_| Region { region_info });
                    } else {
                        pos += region_info.size();
                    }
                } else {
                    // No empty regions
                    break;
                }
            }

            Err(MemoryError::NoRegionAddressSpaceAvailable)
        })
    }

    pub fn deallocate_region(&mut self, region_info: RegionInfo) -> Result<()> {
        unsafe { lock_page_table() }.and_then(|mut page_table| {
            let header_pte = page_table
                .get_pte_for_address(region_info.start_va as u64)
                .and_then(|pte| pte.present().or(Err(MemoryError::InvalidRegion)))
                .and_then(|pp| {
                    PresentRegionHeaderPte::try_from(pp).or(Err(MemoryError::InvalidRegion))
                })?;

            if region_info.start_va + (REGION_CHUNK_SIZE_IN_BYTES * header_pte.size_in_chunks())
                != region_info.limit_va
            {
                return Err(MemoryError::InvalidRegion);
            }

            let (empty_space_start, prev_region_size) = if region_info.start_va > self.base_va {
                let this_prev_header_pte = page_table
                    .get_pte_for_address(region_info.start_va as u64 + PAGE_SIZE)
                    .and_then(|pte| pte.present().or(Err(MemoryError::InvalidRegion)))
                    .and_then(|pp| {
                        PresentRegionHeaderPte::try_from(pp).or(Err(MemoryError::InvalidRegion))
                    })?;

                let prev_region_start = region_info.start_va
                    - (REGION_CHUNK_SIZE_IN_BYTES * this_prev_header_pte.size_in_chunks());

                let prev_prev_header_raw_pte =
                    *page_table.get_pte_for_address(prev_region_start as u64 + PAGE_SIZE)?;

                if prev_prev_header_raw_pte.is_present() {
                    // The previous region is full, so ignore it
                    (
                        region_info.start_va,
                        Some(this_prev_header_pte.size_in_chunks()),
                    )
                } else if prev_region_start == self.base_va {
                    (prev_region_start, None)
                } else {
                    let prev_prev_header_pte = prev_prev_header_raw_pte
                        .not_present()
                        .or(Err(MemoryError::InvalidRegion))
                        .and_then(|pp| {
                            NotPresentRegionHeaderPte::try_from(pp)
                                .or(Err(MemoryError::InvalidRegion))
                        })?;

                    (
                        prev_region_start,
                        Some(prev_prev_header_pte.size_in_chunks()),
                    )
                }
            } else {
                // There are no chunks before us to look at
                (region_info.start_va, None)
            };

            let empty_space_end =
                Self::find_allocated_region(&mut page_table, region_info.limit_va, self.limit_va)?
                    .map(|r| r.start_va)
                    .unwrap_or(self.limit_va);

            // Unmap the region
            self.unmap_region(&mut page_table, &region_info)?;

            // Then write out a full set of clear entries
            let last_empty_chunk_size = Self::write_empty_regions(
                &mut page_table,
                empty_space_start,
                empty_space_end,
                prev_region_size,
            )?
            .unwrap();

            if empty_space_end < self.limit_va {
                // Fix up the previous region pointer in the next region
                Self::update_previous_chunk_size(
                    &mut page_table,
                    empty_space_end,
                    last_empty_chunk_size,
                )
                .expect("Cannot fail after modifying regions");
            }

            Ok(())
        })
    }

    fn map_region(&self, page_table: &mut ActivePageTable, region: &RegionInfo) -> Result<()> {
        let mut flusher = MapperFlushAll::new();
        let mut pos = region.start_va;

        let result: Result<()> = try {
            let original_header_pte: NotPresentRegionHeaderPte = page_table
                .get_pte_for_address(pos as u64)?
                .not_present()
                .expect("Region already mapped")
                .try_into()
                .expect("Invalid region header entry");

            let frame = physmem::allocate_frame().ok_or(MemoryError::OutOfMemory)?;
            flusher.consume(page_table.set_present(
                pos as u64,
                PresentRegionHeaderPte::new(
                    frame,
                    PresentPageFlags::WRITABLE
                        | PresentPageFlags::GLOBAL
                        | PresentPageFlags::NO_EXECUTE,
                    original_header_pte.size_in_chunks(),
                ),
            )?);

            pos += PAGE_SIZE as usize;

            if region.start_va > self.base_va {
                let original_header_pte: NotPresentRegionHeaderPte = page_table
                    .get_pte_for_address(pos as u64)?
                    .not_present()
                    .expect("Region already mapped")
                    .try_into()
                    .expect("Invalid region header entry");

                let frame = physmem::allocate_frame().ok_or(MemoryError::OutOfMemory)?;
                flusher.consume(page_table.set_present(
                    pos as u64,
                    PresentRegionHeaderPte::new(
                        frame,
                        PresentPageFlags::WRITABLE
                            | PresentPageFlags::GLOBAL
                            | PresentPageFlags::NO_EXECUTE,
                        original_header_pte.size_in_chunks(),
                    ),
                )?);

                pos += PAGE_SIZE as usize;
            }

            while pos < region.limit_va {
                let frame = physmem::allocate_frame().ok_or(MemoryError::OutOfMemory)?;

                flusher.consume(page_table.map_to(
                    pos as u64,
                    frame,
                    PresentPageFlags::WRITABLE
                        | PresentPageFlags::GLOBAL
                        | PresentPageFlags::NO_EXECUTE,
                )?);
                pos += PAGE_SIZE as usize;
            }
        };

        flusher.flush(page_table);

        if let Err(error) = result {
            self.unmap_region(page_table, region)
                .expect("Failed to unmap region");
            Err(error)
        } else {
            Ok(())
        }
    }

    fn unmap_region(&self, page_table: &mut ActivePageTable, region: &RegionInfo) -> Result<()> {
        let mut flusher = MapperFlushAll::new();
        let mut pos = region.start_va;

        let original_header_pte: PresentRegionHeaderPte = page_table
            .get_pte_for_address(pos as u64)?
            .present()
            .expect("Region not mapped")
            .try_into()
            .expect("Invalid region header entry");

        flusher.consume(page_table.unmap_and_free_and_replace(
            pos,
            NotPresentRegionHeaderPte::new(original_header_pte.size_in_chunks()),
        )?);

        pos += PAGE_SIZE as usize;

        if region.start_va > self.base_va {
            let original_header_pte: PresentRegionHeaderPte = page_table
                .get_pte_for_address(pos as u64)?
                .present()
                .expect("Region not mapped")
                .try_into()
                .expect("Invalid region header entry");

            flusher.consume(page_table.unmap_and_free_and_replace(
                pos,
                NotPresentRegionHeaderPte::new(original_header_pte.size_in_chunks()),
            )?);

            pos += PAGE_SIZE as usize;
        }

        while pos < region.limit_va {
            flusher.consume(page_table.unmap_and_free(pos)?);
            pos += PAGE_SIZE as usize;
        }

        flusher.flush(page_table);

        Ok(())
    }

    fn find_empty_region(
        page_table: &mut ActivePageTable,
        start_va: usize,
        limit_va: usize,
    ) -> Result<Option<RegionInfo>> {
        let mut pos = start_va;
        while pos < limit_va {
            let header_pte = page_table.get_pte_for_address(pos as u64)?;

            if header_pte.is_present() {
                let header_pte = PresentRegionHeaderPte::try_from(header_pte.present().unwrap())
                    .expect("invalid page table entry in region header");
                let region_size_in_chunks = header_pte.size_in_chunks();

                pos += region_size_in_chunks * REGION_CHUNK_SIZE_IN_BYTES;
            } else {
                let header_pte =
                    NotPresentRegionHeaderPte::try_from(header_pte.not_present().unwrap())
                        .expect("invalid page table entry in region header");
                let region_size_in_chunks = header_pte.size_in_chunks();

                return Ok(Some(RegionInfo {
                    start_va: pos,
                    limit_va: pos + (region_size_in_chunks * REGION_CHUNK_SIZE_IN_BYTES),
                }));
            }
        }

        Ok(None)
    }

    fn find_allocated_region(
        page_table: &mut ActivePageTable,
        start_va: usize,
        limit_va: usize,
    ) -> Result<Option<RegionInfo>> {
        let mut pos = start_va;
        while pos < limit_va {
            let header_pte = page_table.get_pte_for_address(pos as u64)?;

            if header_pte.is_present() {
                let header_pte = PresentRegionHeaderPte::try_from(header_pte.present().unwrap())
                    .expect("invalid page table entry in region header");
                let region_size_in_chunks = header_pte.size_in_chunks();

                return Ok(Some(RegionInfo {
                    start_va: pos,
                    limit_va: pos + (region_size_in_chunks * REGION_CHUNK_SIZE_IN_BYTES),
                }));
            } else {
                let header_pte =
                    NotPresentRegionHeaderPte::try_from(header_pte.not_present().unwrap())
                        .expect("invalid page table entry in region header");
                let region_size_in_chunks = header_pte.size_in_chunks();

                pos += region_size_in_chunks * REGION_CHUNK_SIZE_IN_BYTES;
            }
        }

        Ok(None)
    }

    fn write_empty_regions(
        page_table: &mut ActivePageTable,
        start_va: usize,
        limit_va: usize,
        mut prev_chunk_size: Option<usize>,
    ) -> Result<Option<usize>> {
        // The P4 requirement is important
        assert_eq!(
            p4_index(start_va as u64),
            p4_index((limit_va - 1) as u64),
            "Heap region space cannot span multiple P4 entries"
        );
        assert_eq!(
            p3_index(start_va as u64),
            p3_index((limit_va - 1) as u64),
            "Heap region space cannot span multiple P3 entries"
        );
        assert!(
            limit_va >= start_va,
            "Limit address is before start address"
        );
        assert_eq!(
            align_up(start_va, REGION_CHUNK_SIZE_IN_BYTES),
            start_va,
            "Start address is not region size aligned"
        );
        assert_eq!(
            align_down(limit_va, REGION_CHUNK_SIZE_IN_BYTES),
            limit_va,
            "Limit address is not region size aligned"
        );

        let p2 = page_table
            .p4_mut()
            .create_next_table(p4_index(start_va as u64))?
            .create_next_table(p3_index(start_va as u64))?;

        let mut flusher = MapperFlushAll::new();
        let mut pos = start_va;

        while pos < limit_va {
            let empty_region_size_in_chunks =
                ((limit_va - pos) / REGION_CHUNK_SIZE_IN_BYTES).min(MAXIMUM_REGION_SIZE_IN_CHUNKS);
            let empty_region_size_in_bytes =
                empty_region_size_in_chunks * REGION_CHUNK_SIZE_IN_BYTES;
            let chunk_end = pos + empty_region_size_in_bytes;

            let mut current_p2_index = p2_index(pos as u64);

            // Write out the header
            flusher.consume(page_table.set_not_present(
                pos as u64,
                NotPresentRegionHeaderPte::new(empty_region_size_in_chunks),
            )?);
            pos += PAGE_SIZE as usize;

            if let Some(prev_chunk_size) = prev_chunk_size {
                flusher.consume(page_table.set_not_present(
                    pos as u64,
                    NotPresentRegionHeaderPte::new(prev_chunk_size),
                )?);
                pos += PAGE_SIZE as usize;
            }

            // Set the prev chunk size
            prev_chunk_size = Some(empty_region_size_in_chunks);

            loop {
                // Loop through the current p1 table to make sure we set everything to unused
                while pos < chunk_end && p2_index(pos as u64) == current_p2_index {
                    flusher.consume(
                        page_table.set_not_present(pos as u64, RawNotPresentPte::unused())?,
                    );
                    pos += PAGE_SIZE as usize;
                }

                if pos == chunk_end {
                    // All done
                    break;
                }

                // We're moving to a new p1 table. We can skip it if it doesn't exist
                if p2.next_table_frame(p2_index(pos as u64)).is_ok() {
                    // The p1 table does exist, so we're going to need to clear it
                    current_p2_index = p2_index(pos as u64);
                } else {
                    // Move the position and go around again
                    pos = (pos + (512 * PAGE_SIZE as usize)).min(chunk_end);
                }
            }
        }

        // We don't need to flush the tlb here because we haven't changed any visibility
        unsafe {
            flusher.ignore();
        }

        Ok(prev_chunk_size)
    }

    fn update_this_chunk_size(
        page_table: &mut ActivePageTable,
        start_va: usize,
        this_size_in_chunks: usize,
    ) -> Result<()> {
        let header_address = start_va;
        let mut header_pte = page_table.get_pte_mut_for_address(header_address as u64)?;

        if header_pte.is_present() {
            let original_header_pte =
                PresentRegionHeaderPte::try_from(header_pte.present().unwrap())
                    .expect("invalid page table entry in region header");
            *header_pte = RawPresentPte::from(PresentRegionHeaderPte::new(
                original_header_pte.frame(),
                original_header_pte.flags(),
                this_size_in_chunks,
            ))
            .into();
        } else {
            let _original_header_pte =
                NotPresentRegionHeaderPte::try_from(header_pte.not_present().unwrap())
                    .expect("invalid page table entry in region header");
            *header_pte =
                RawNotPresentPte::from(NotPresentRegionHeaderPte::new(this_size_in_chunks)).into();
        }

        Ok(())
    }

    fn update_previous_chunk_size(
        page_table: &mut ActivePageTable,
        start_va: usize,
        prev_size_in_chunks: usize,
    ) -> Result<()> {
        let header_address = start_va + (PAGE_SIZE as usize);
        let mut header_pte = page_table.get_pte_mut_for_address(header_address as u64)?;

        if header_pte.is_present() {
            let original_header_pte =
                PresentRegionHeaderPte::try_from(header_pte.present().unwrap())
                    .expect("invalid page table entry in region header");
            *header_pte = RawPresentPte::from(PresentRegionHeaderPte::new(
                original_header_pte.frame(),
                original_header_pte.flags(),
                prev_size_in_chunks,
            ))
            .into();
        } else {
            let _original_header_pte =
                NotPresentRegionHeaderPte::try_from(header_pte.not_present().unwrap())
                    .expect("invalid page table entry in region header");
            *header_pte =
                RawNotPresentPte::from(NotPresentRegionHeaderPte::new(prev_size_in_chunks)).into();
        }

        Ok(())
    }
}

pub struct Region {
    region_info: RegionInfo,
}

impl Region {
    pub fn as_ptr<T>(&self) -> *const T {
        self.as_ptr_offset(0)
    }

    pub fn as_ptr_offset<T>(&self, offset: usize) -> *const T {
        (self.region_info.start_va + offset) as *const T
    }
    pub fn as_mut_ptr<T>(&mut self) -> *mut T {
        self.as_mut_ptr_offset(0)
    }

    pub fn as_mut_ptr_offset<T>(&mut self, offset: usize) -> *mut T {
        (self.region_info.start_va + offset) as *mut T
    }

    pub fn start(&self) -> usize {
        self.region_info.start_va
    }

    pub fn limit(&self) -> usize {
        self.region_info.limit_va
    }
}

impl Drop for Region {
    fn drop(&mut self) {
        REGION_MANAGER
            .lock()
            .deallocate_region(self.region_info)
            .expect("Failed to deallocate region");
    }
}

pub unsafe fn init(base: usize, limit: usize) -> Result<()> {
    REGION_MANAGER.init(RegionManager::new(base, limit)?);
    Ok(())
}

pub fn allocate_region(pages: usize) -> Result<Region> {
    REGION_MANAGER.lock().allocate_region(pages)
}
