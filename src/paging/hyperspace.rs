use super::{
    p1_index, p2_index, p3_index, p4_index, Frame, MemoryError, PageFlags, PageTable,
    PageTableEntry, PageTableIndex, Result, HYPERSPACE_BASE, L1, L4, PAGE_SIZE,
};
use bootloader::BootInfo;
use core::convert::TryFrom;
use core::mem::MaybeUninit;
use core::ops::{Deref, DerefMut};
use spin::Mutex;
use x86::tlb;

pub struct HyperspaceMapping {
    va: u64,
}

impl HyperspaceMapping {
    pub fn as_ptr<T>(&self) -> *const T {
        self.va as *const T
    }

    pub fn as_mut_ptr<T>(&self) -> *mut T {
        self.va as *mut T
    }

    pub fn release(self) {
        core::mem::drop(self);
    }
}

impl Drop for HyperspaceMapping {
    fn drop(&mut self) {
        lock_hyperspace().unmap_page(self.va);
    }
}

struct HyperspaceMapper {
    page_table: Option<&'static mut PageTable<L1>>,
}

impl HyperspaceMapper {
    pub fn init_post_paging(&mut self) {
        self.page_table = Some(unsafe { &mut *(HYPERSPACE_BASE as *mut PageTable<L1>) });
    }

    pub fn map_page(&mut self, frame: Frame) -> Result<HyperspaceMapping> {
        self.page_table_mut()
            .iter_mut()
            .enumerate()
            .find(|(_, e)| !e.flags().contains(PageFlags::PRESENT))
            .map(|(index, e)| {
                let va = HYPERSPACE_BASE + (index as u64 * PAGE_SIZE);
                *e = PageTableEntry::from_frame_and_flags(
                    frame,
                    PageFlags::PRESENT | PageFlags::GLOBAL | PageFlags::WRITABLE,
                );
                unsafe { tlb::flush(va as usize) };

                HyperspaceMapping { va }
            })
            .ok_or(MemoryError::OutOfHyperspacePages)
    }

    pub fn unmap_page(&mut self, va: u64) {
        assert!((va & (PAGE_SIZE - 1)) == 0);
        assert!(va > HYPERSPACE_BASE && va < self.limit());

        let index = PageTableIndex::try_from((va - HYPERSPACE_BASE) / PAGE_SIZE).unwrap();
        let page_table = self.page_table_mut();

        assert!(page_table[index].flags().contains(PageFlags::PRESENT));
        page_table[index].set_unused();
        unsafe { tlb::flush(va as usize) };
    }

    fn page_table_mut(&mut self) -> &mut PageTable<L1> {
        self.page_table.as_mut().unwrap()
    }

    const fn limit(&self) -> u64 {
        HYPERSPACE_BASE + (512 * 4096)
    }
}

static HYPERSPACE: Mutex<MaybeUninit<HyperspaceMapper>> = Mutex::new(MaybeUninit::uninit());

#[repr(transparent)]
struct HyperspaceLock<'a> {
    guard: spin::MutexGuard<'a, MaybeUninit<HyperspaceMapper>>,
}

impl<'a> Deref for HyperspaceLock<'a> {
    type Target = HyperspaceMapper;
    fn deref(&self) -> &Self::Target {
        unsafe { &*(self.guard.as_ptr() as *const HyperspaceMapper) }
    }
}

impl<'a> DerefMut for HyperspaceLock<'a> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe { &mut *(self.guard.as_ptr() as *mut HyperspaceMapper) }
    }
}

fn lock_hyperspace<'a>() -> HyperspaceLock<'a> {
    HyperspaceLock {
        guard: HYPERSPACE.lock(),
    }
}

pub unsafe fn init(boot_info: &BootInfo, page_table: &'static mut PageTable<L4>) {
    use super::table::BootPageTable;

    assert!(u16::from(p1_index(HYPERSPACE_BASE)) == 0);

    let (l1_table, l1_table_frame) = {
        let l2_table = page_table
            .boot_create_next_table(boot_info, p4_index(HYPERSPACE_BASE))
            .boot_create_next_table(boot_info, p3_index(HYPERSPACE_BASE));

        l2_table.boot_create_next_table(boot_info, p2_index(HYPERSPACE_BASE));

        let l1_table_frame = l2_table
            .next_table_frame(p2_index(HYPERSPACE_BASE))
            .unwrap();

        (
            l2_table
                .boot_next_table_mut(boot_info, p2_index(HYPERSPACE_BASE))
                .unwrap(),
            l1_table_frame,
        )
    };

    assert!(l1_table
        .iter()
        .find(|e| e.flags().contains(PageFlags::PRESENT))
        .is_none());

    // Most important thing - we need to map the hyperspace page table into hyperspace. We defer
    // generating the reference to it until the page table is mapped
    l1_table[PageTableIndex::new_unchecked(0)] = PageTableEntry::from_frame_and_flags(
        l1_table_frame,
        PageFlags::GLOBAL | PageFlags::PRESENT | PageFlags::WRITABLE,
    );

    HYPERSPACE
        .lock()
        .as_mut_ptr()
        .write(HyperspaceMapper { page_table: None });
}

pub fn init_post_paging() {
    lock_hyperspace().init_post_paging();
}

pub fn map_page(frame: Frame) -> Result<HyperspaceMapping> {
    lock_hyperspace().map_page(frame)
}
