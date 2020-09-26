use super::page_entry::PresentPageFlags;
use super::{
    lock_page_table, page_entry, ActivePageTable, Frame, MapperFlushAll, MemoryError, Result,
    PAGE_SIZE,
};
use crate::init_mutex::InitMutex;
use crate::physmem;
use bitflags::bitflags;

bitflags! {
    pub struct PhysicalMappingFlags: u64 {
        const UNCACHED = 1 << 0;
        const READ_ONLY = 1 << 1;
    }
}

impl From<PhysicalMappingFlags> for PresentPageFlags {
    fn from(pmf: PhysicalMappingFlags) -> Self {
        let mut ret = PresentPageFlags::GLOBAL | PresentPageFlags::NO_EXECUTE;

        if !pmf.contains(PhysicalMappingFlags::READ_ONLY) {
            ret |= PresentPageFlags::WRITABLE;
        }

        if pmf.contains(PhysicalMappingFlags::UNCACHED) {
            ret |= PresentPageFlags::NO_CACHE;
        }

        ret
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PhysicalMapping {
    physical_address: usize,
    flags: PhysicalMappingFlags,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RegionType {
    Free,
    Heap,
    KernelStack,
    PhysicalMapping(PhysicalMapping),
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct RegionMapEntry {
    base: usize,
    limit: usize,
    region_type: Option<RegionType>,
}

impl RegionMapEntry {
    pub const fn empty() -> Self {
        Self {
            base: 0,
            limit: 0,
            region_type: None,
        }
    }

    pub fn size(&self) -> usize {
        self.limit - self.base
    }

    pub fn region_info(&self) -> RegionInfo {
        RegionInfo {
            start_va: self.base,
            limit_va: self.limit,
        }
    }
}

#[repr(C)]
struct RegionMapPageHeader {
    frame: Option<Frame>,
    next_entry: Option<&'static mut RegionMapPage>,
}

const REGION_MAP_ENTRIES_IN_PAGE: usize = (PAGE_SIZE as usize
    - align_up(
        core::mem::size_of::<RegionMapPageHeader>(),
        core::mem::align_of::<RegionMapEntry>(),
    ))
    / core::mem::size_of::<RegionMapEntry>();

#[repr(C)]
struct RegionMapPage {
    header: RegionMapPageHeader,
    entries: [RegionMapEntry; REGION_MAP_ENTRIES_IN_PAGE],
}

impl RegionMapPage {
    pub fn from_frame(frame: Frame) -> Self {
        Self {
            header: RegionMapPageHeader {
                frame: Some(frame),
                next_entry: None,
            },
            entries: [RegionMapEntry::empty(); REGION_MAP_ENTRIES_IN_PAGE],
        }
    }
}

const fn align_down(addr: usize, align: usize) -> usize {
    if align.is_power_of_two() {
        addr & !(align - 1)
    } else if align == 0 {
        addr
    } else {
        panic!("`align` must be a power of 2");
    }
}

const fn align_up(addr: usize, align: usize) -> usize {
    align_down(addr + align - 1, align)
}

#[derive(Debug, Clone, Copy)]
struct RegionInfo {
    start_va: usize,
    limit_va: usize,
}

impl RegionInfo {
    pub fn size(&self) -> usize {
        self.limit_va - self.start_va
    }
}

struct RegionManager {
    head_page: RegionMapPage,
}

impl RegionManager {
    pub fn new(base: usize, limit: usize) -> Self {
        let mut entries = [RegionMapEntry {
            base: 0,
            limit: 0,
            region_type: None,
        }; REGION_MAP_ENTRIES_IN_PAGE];

        entries[0] = RegionMapEntry {
            base,
            limit,
            region_type: Some(RegionType::Free),
        };

        Self {
            head_page: RegionMapPage {
                header: RegionMapPageHeader {
                    frame: None,
                    next_entry: None,
                },
                entries,
            },
        }
    }

    pub fn allocate_region(&mut self, pages: usize, region_type: RegionType) -> Result<Region> {
        let required_size = pages * PAGE_SIZE as usize;
        let ret = Self::allocate_first_fit(&mut self.head_page, required_size, |entry| {
            debug_assert_eq!(
                entry.size(),
                required_size,
                "allocate_first_fit returned wrong size region"
            );
            debug_assert_eq!(
                entry.region_type.unwrap(),
                RegionType::Free,
                "allocate_first_fit returned incorrect region type"
            );

            Self::map_region(entry, region_type)?;
            Ok(region_type)
        })
        .map(|region_info| Region::new(region_info));
        Self::print_entries(&self.head_page);
        ret
    }

    fn allocate_first_fit(
        mut this_page: &mut RegionMapPage,
        required_size: usize,
        mapper: impl FnOnce(&RegionMapEntry) -> Result<RegionType>,
    ) -> Result<RegionInfo> {
        loop {
            for i in 0..REGION_MAP_ENTRIES_IN_PAGE {
                match this_page.entries[i].region_type {
                    None => {
                        assert!(
                            this_page.header.next_entry.is_none(),
                            "Unexpected untyped entry"
                        );
                        return Err(MemoryError::NoRegionAddressSpaceAvailable);
                    }

                    Some(RegionType::Free) if this_page.entries[i].size() > required_size => {
                        // We might need a frame to extend the table. We allocate one now so that we know that
                        // we don't have to worry about that failure mode later. This has to be a kernel frame because we
                        // depend on it already being mapped
                        let table_frame =
                            physmem::allocate_kernel_frame().ok_or(MemoryError::OutOfMemory)?;

                        let last_entry = RegionMapEntry {
                            base: this_page.entries[i].base + required_size,
                            limit: this_page.entries[i].limit,
                            region_type: Some(RegionType::Free),
                        };
                        this_page.entries[i].limit = this_page.entries[i].base + required_size;

                        // Do the mapping before shuffling the page entries. This is safe as long as the mapper does
                        // not recurse, which it can't
                        let region_type = match mapper(&this_page.entries[i]) {
                            Ok(region_type) => region_type,

                            Err(e) => {
                                // We need to put the region back as it was. This is why we did the mapping before the
                                // shuffle because the only thing we need to reverse is the size change
                                this_page.entries[i].limit = last_entry.limit;
                                physmem::deallocate_frame(table_frame);
                                return Err(e);
                            }
                        };

                        this_page.entries[i].region_type = Some(region_type);

                        let mut table_frame = Some(table_frame);

                        Self::shuffle_entries_up(this_page, i + 1, last_entry, &mut table_frame);

                        // If we didn't use the frame, we can free it
                        if let Some(unused_frame) = table_frame {
                            physmem::deallocate_frame(unused_frame);
                        }

                        return Ok(this_page.entries[i].region_info());
                    }

                    Some(RegionType::Free) if this_page.entries[i].size() == required_size => {
                        // We've found a region that is exactly the right size, so all we need to do is map it
                        let region_type = mapper(&this_page.entries[i])?;
                        this_page.entries[i].region_type = Some(region_type);

                        return Ok(this_page.entries[i].region_info());
                    }

                    Some(_) => {}
                }
            }

            if this_page.header.next_entry.is_none() {
                return Err(MemoryError::NoRegionAddressSpaceAvailable);
            } else {
                this_page = this_page.header.next_entry.as_mut().unwrap();
            }
        }
    }

    fn shuffle_entries_up(
        mut this_page: &mut RegionMapPage,
        start_index: usize,
        mut fill_entry: RegionMapEntry,
        table_frame: &mut Option<Frame>,
    ) {
        let mut pos = start_index;
        loop {
            // Move all of the entries up one
            while fill_entry.region_type.is_some() && pos < REGION_MAP_ENTRIES_IN_PAGE {
                core::mem::swap(&mut fill_entry, &mut this_page.entries[pos]);
                pos += 1;
            }

            if fill_entry.region_type.is_none() {
                break;
            } else {
                use crate::println;

                if this_page.header.next_entry.is_none() {
                    println!("Need to allocate a new page {}", pos);

                    this_page.header.next_entry = Some(unsafe {
                        let new_frame = table_frame.take().expect("No table frame provided");
                        let new_page_ptr: *mut RegionMapPage =
                            super::phys_to_virt_mut(new_frame.physical_address() as usize);

                        new_page_ptr.write(RegionMapPage::from_frame(new_frame));
                        &mut *new_page_ptr
                    });
                }

                println!("Moving to next page");
                this_page = this_page.header.next_entry.as_mut().unwrap();
                pos = 0;
            }
        }
    }

    fn map_region(region_entry: &RegionMapEntry, region_type: RegionType) -> Result<()> {
        debug_assert_eq!(
            region_entry.region_type.unwrap(),
            RegionType::Free,
            "map_region can only be used on free regions"
        );

        match region_type {
            RegionType::Heap => Self::map_nonpaged(region_entry.base, region_entry.limit)?,
            RegionType::KernelStack => {
                Self::map_kernel_stack(region_entry.base, region_entry.limit)?
            }
            RegionType::PhysicalMapping(physical_mapping) => {
                Self::map_physical_memory(&physical_mapping, region_entry.base, region_entry.limit)?
            }

            RegionType::Free => panic!("Cannot map free region"),
        }

        Ok(())
    }

    fn map_nonpaged_impl(
        page_table: &mut ActivePageTable,
        flusher: &mut MapperFlushAll,
        base: usize,
        limit: usize,
        unmap_base: usize,
        unmap_limit: usize,
    ) -> Result<()> {
        let allocate_result: Result<()> = try {
            let pages = (limit - base) / PAGE_SIZE as usize;
            for page in 0..pages {
                let page_addr = base + (page * PAGE_SIZE as usize);
                // We can use user frames here since we're mapping them
                let frame = physmem::allocate_user_frame().ok_or(MemoryError::OutOfMemory)?;

                flusher.consume(page_table.map_to(
                    page_addr,
                    frame,
                    PresentPageFlags::WRITABLE
                        | PresentPageFlags::GLOBAL
                        | PresentPageFlags::NO_EXECUTE,
                )?);
            }
        };

        if allocate_result.is_err() {
            Self::unmap_nonpaged(unmap_base, unmap_limit, true);
        }

        allocate_result
    }

    fn map_nonpaged(base: usize, limit: usize) -> Result<()> {
        debug_assert!(limit > base, "Invalid range");
        debug_assert_eq!(
            base,
            align_up(base, PAGE_SIZE as usize),
            "base address is not page aligned"
        );
        debug_assert_eq!(
            limit,
            align_down(limit, PAGE_SIZE as usize),
            "limit address is not page aligned"
        );

        let mut page_table = unsafe { lock_page_table() };
        let mut flusher = MapperFlushAll::new();

        let result =
            Self::map_nonpaged_impl(&mut page_table, &mut flusher, base, limit, base, limit);

        flusher.flush(&mut page_table);

        result
    }

    fn map_kernel_stack(base: usize, limit: usize) -> Result<()> {
        debug_assert!(limit > base + PAGE_SIZE, "Invalid range");
        debug_assert_eq!(
            base,
            align_up(base, PAGE_SIZE as usize),
            "base address is not page aligned"
        );
        debug_assert_eq!(
            limit,
            align_down(limit, PAGE_SIZE as usize),
            "limit address is not page aligned"
        );

        let mut page_table = unsafe { lock_page_table() };
        let mut flusher = MapperFlushAll::new();

        let result = try {
            flusher.consume(
                page_table.set_not_present(base, page_entry::KernelStackGuardPagePte::new())?,
            );
            Self::map_nonpaged_impl(
                &mut page_table,
                &mut flusher,
                base + PAGE_SIZE,
                limit,
                base,
                limit,
            )?;
        };

        flusher.flush(&mut page_table);
        result
    }

    fn map_physical_memory(
        physical_mapping: &PhysicalMapping,
        base: usize,
        limit: usize,
    ) -> Result<()> {
        debug_assert!(limit > base, "Invalid range");
        debug_assert_eq!(
            base,
            align_up(base, PAGE_SIZE as usize),
            "base address is not page aligned"
        );
        debug_assert_eq!(
            limit,
            align_down(limit, PAGE_SIZE as usize),
            "limit address is not page aligned"
        );

        let mut page_table = unsafe { lock_page_table() };
        let mut flusher = MapperFlushAll::new();

        let result = try {
            let pages = (limit - base) / PAGE_SIZE as usize;
            for page in 0..pages {
                let page_addr = base + (page * PAGE_SIZE as usize);
                // We can use user frames here since we're mapping them
                let frame = Frame::containing_address(
                    physical_mapping.physical_address + (page * PAGE_SIZE),
                );

                flusher.consume(page_table.map_to(
                    page_addr,
                    frame,
                    physical_mapping.flags.into(),
                )?);
            }
        };

        flusher.flush(&mut page_table);
        result
    }

    pub fn deallocate_region(&mut self, region_info: &RegionInfo) {
        Self::deallocate_recurse_thing(&mut self.head_page, region_info);
        Self::print_entries(&self.head_page);
    }

    fn deallocate_recurse_thing<'a>(
        mut this_page: &'a mut RegionMapPage,
        region_info: &RegionInfo,
    ) {
        use crate::println;

        loop {
            for j in 0..REGION_MAP_ENTRIES_IN_PAGE {
                println!(
                    "region_info.start_va: {:#x} region: {:#x} {:?}",
                    region_info.start_va,
                    this_page.entries[j].base,
                    this_page.entries[j].region_type
                );

                assert!(
                    this_page.entries[j].base <= region_info.start_va,
                    "Attempting to free invalid region"
                );
                assert!(
                    this_page.entries[j].region_type.is_some(),
                    "Attempting to free invalid region"
                );

                let drop_region_info = if this_page.entries[j].limit == region_info.start_va
                    && this_page.entries[j].region_type.unwrap() == RegionType::Free
                {
                    let lead_bytes = this_page.entries[j].size();

                    if j + 1 < REGION_MAP_ENTRIES_IN_PAGE {
                        // We could check here whether the next region is good, but there is no need - it
                        // will be checked later before we free it.
                        this_page.entries[j] = this_page.entries[j + 1];
                        Self::shuffle_entries_down(this_page, j + 1);
                    } else {
                        let next_page = this_page.header.next_entry.as_mut().unwrap();
                        // We could check here whether the next region is good, but there is no need - it
                        // will be checked later before we free it.
                        this_page.entries[j] = next_page.entries[0];
                        Self::shuffle_entries_down(next_page, 0);
                    }

                    Some((j, lead_bytes))
                } else if this_page.entries[j].base == region_info.start_va {
                    Some((j, 0))
                } else {
                    None
                };

                if let Some((drop_region_index, lead_bytes)) = drop_region_info {
                    assert_ne!(
                        this_page.entries[drop_region_index].region_type.unwrap(),
                        RegionType::Free,
                        "Attempting to free invalid region"
                    );
                    assert_eq!(
                        this_page.entries[drop_region_index].limit, region_info.limit_va,
                        "Attempting to free invalid region"
                    );

                    Self::unmap_region(&this_page.entries[drop_region_index]);

                    let tail_bytes = if drop_region_index + 1 < REGION_MAP_ENTRIES_IN_PAGE {
                        if this_page.entries[drop_region_index + 1].region_type
                            == Some(RegionType::Free)
                        {
                            let tail_bytes = this_page.entries[drop_region_index + 1].size();
                            Self::shuffle_entries_down(this_page, drop_region_index + 1);
                            tail_bytes
                        } else {
                            // The next entry is not free so leave it alone
                            0
                        }
                    } else if this_page.header.next_entry.is_some() {
                        // This is the last entry of this page, but there is another page after
                        let next_page = this_page.header.next_entry.as_mut().unwrap();
                        if next_page.entries[0].region_type == Some(RegionType::Free) {
                            let tail_bytes = next_page.entries[0].size();
                            Self::shuffle_entries_down(next_page, 0);
                            tail_bytes
                        } else {
                            // The next entry is not free so leave it alone
                            0
                        }
                    } else {
                        // There are no entries after this one, so no extra bytes
                        0
                    };

                    println!(
                        "Freeing region at {:#x} - {} in region {} lead {} tail",
                        region_info.start_va,
                        region_info.size(),
                        lead_bytes,
                        tail_bytes
                    );

                    // The region is already unmapped at this point, so we just need to fix up the limit
                    this_page.entries[drop_region_index].base -= lead_bytes;
                    this_page.entries[drop_region_index].limit += tail_bytes;
                    this_page.entries[drop_region_index].region_type = Some(RegionType::Free);
                    return;
                }
            }

            assert!(
                this_page.header.next_entry.is_some(),
                "Attempting to free an invalid region"
            );
            this_page = this_page.header.next_entry.as_mut().unwrap();
        }
    }

    fn shuffle_entries_down(mut this_page: &mut RegionMapPage, region_index: usize) {
        use crate::println;
        let mut pos = region_index;
        loop {
            if pos == REGION_MAP_ENTRIES_IN_PAGE - 1 {
                println!("MOVING TO NEXT PAGE");
                if this_page.header.next_entry.is_none() {
                    this_page.entries[pos] = RegionMapEntry::empty();
                    return;
                }

                // Otherwise get the entry from the next page
                this_page.entries[pos] = this_page.header.next_entry.as_ref().unwrap().entries[0];

                if this_page.entries[pos].region_type.is_none() {
                    // We've empties the next page, so we can free it
                    let next_page = this_page.header.next_entry.take();
                    let next_page_ref = next_page.as_ref().unwrap();
                    if let Some(frame) = next_page_ref.header.frame {
                        println!("FREE PAGE {:#x}", frame.physical_address());
                        physmem::deallocate_frame(frame);
                    }

                    return;
                }

                this_page = this_page.header.next_entry.as_mut().unwrap();
                pos = 0;
            } else {
                this_page.entries[pos] = this_page.entries[pos + 1];
                if this_page.entries[pos].region_type.is_none() {
                    // We're done
                    return;
                }

                pos += 1;
            }
        }
    }

    fn unmap_region(region_entry: &RegionMapEntry) {
        debug_assert_ne!(
            region_entry.region_type.unwrap(),
            RegionType::Free,
            "unmap_region cannot be used on free regions"
        );

        match region_entry.region_type.unwrap() {
            RegionType::Heap | RegionType::KernelStack => {
                Self::unmap_nonpaged(region_entry.base, region_entry.limit, true)
            }
            RegionType::PhysicalMapping(_) => {
                Self::unmap_nonpaged(region_entry.base, region_entry.limit, false)
            }

            RegionType::Free => panic!("Cannot unmap free region"),
        }
    }

    fn unmap_nonpaged(base: usize, limit: usize, free_pages: bool) {
        debug_assert!(limit > base, "Invalid range");
        debug_assert_eq!(
            base,
            align_up(base, PAGE_SIZE as usize),
            "base address is not page aligned"
        );
        debug_assert_eq!(
            limit,
            align_down(limit, PAGE_SIZE as usize),
            "limit address is not page aligned"
        );

        let mut page_table = unsafe { lock_page_table() };
        let mut flusher = MapperFlushAll::new();

        let pages = (limit - base) / PAGE_SIZE as usize;
        for page in 0..pages {
            let page_addr = base + (page * PAGE_SIZE as usize);

            flusher.consume(page_table.unmap(page_addr, free_pages));
        }

        flusher.flush(&mut page_table);
    }

    fn print_entries(mut this_page: &RegionMapPage) {
        let mut pos = 0;
        loop {
            if pos == REGION_MAP_ENTRIES_IN_PAGE {
                match this_page.header.next_entry.as_ref() {
                    None => return,
                    Some(next) => {
                        this_page = next;
                        pos = 0;
                    }
                }
            }
            use crate::println;
            println!(
                "REGION {:#x} - {:#x} {:?}",
                this_page.entries[pos].base,
                this_page.entries[pos].limit,
                this_page.entries[pos].region_type
            );

            if this_page.entries[pos].region_type.is_none() {
                return;
            }

            pos += 1;
        }
    }
}

static REGION_MANAGER: InitMutex<RegionManager> = InitMutex::new();

#[derive(Debug)]
pub struct Region {
    region_info: RegionInfo,
    sub_region_offset: usize,
    sub_region_length: usize,
}

impl Region {
    fn new(region_info: RegionInfo) -> Self {
        Self {
            region_info,
            sub_region_offset: 0,
            sub_region_length: region_info.size(),
        }
    }

    pub fn apply_offset(self, offset: usize, length: usize) -> Self {
        let md = core::mem::ManuallyDrop::new(self);

        assert!(offset + length <= md.size());
        Self {
            region_info: md.region_info,
            sub_region_offset: md.sub_region_offset + offset,
            sub_region_length: length,
        }
    }

    pub fn as_ptr<T>(&self) -> *const T {
        self.as_ptr_offset(0)
    }

    pub fn as_ptr_offset<T>(&self, offset: usize) -> *const T {
        (self.start() + offset) as *const T
    }
    pub fn as_mut_ptr<T>(&mut self) -> *mut T {
        self.as_mut_ptr_offset(0)
    }

    pub fn as_mut_ptr_offset<T>(&mut self, offset: usize) -> *mut T {
        (self.start() + offset) as *mut T
    }

    pub fn start(&self) -> usize {
        self.region_info.start_va + self.sub_region_offset
    }

    pub fn limit(&self) -> usize {
        self.start() + self.size()
    }

    pub fn size(&self) -> usize {
        self.sub_region_length
    }
}

impl Drop for Region {
    fn drop(&mut self) {
        REGION_MANAGER.lock().deallocate_region(&self.region_info);
    }
}

pub use super::kernel_stack::KernelStack;

pub unsafe fn init(base: usize, limit: usize) {
    REGION_MANAGER.init(RegionManager::new(base, limit));
}

pub fn allocate_region(pages: usize) -> Result<Region> {
    REGION_MANAGER
        .lock()
        .allocate_region(pages, RegionType::Heap)
}

pub fn allocate_kernel_stack(pages: usize) -> Result<KernelStack> {
    REGION_MANAGER
        .lock()
        .allocate_region(pages, RegionType::KernelStack)
        .map(|region| KernelStack::new(region))
}

pub unsafe fn map_physical_memory(
    physical_address: usize,
    size: usize,
    flags: PhysicalMappingFlags,
) -> Result<Region> {
    let aligned_start = align_down(physical_address, PAGE_SIZE);
    let aligned_limit = align_up(physical_address + size, PAGE_SIZE);
    let pages = (aligned_limit - aligned_start) / PAGE_SIZE;
    let offset = physical_address - aligned_start;

    REGION_MANAGER
        .lock()
        .allocate_region(
            pages,
            RegionType::PhysicalMapping(PhysicalMapping {
                physical_address: aligned_start,
                flags,
            }),
        )
        .map(|region| region.apply_offset(offset, size))
}
