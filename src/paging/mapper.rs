use super::page_entry::{self, PresentPageFlags, RawNotPresentPte, RawPresentPte, RawPte};
use super::{
    map_page, p1_index, p2_index, p3_index, p4_index, ActivePageTable, HierarchyLevel,
    HyperspaceMapping, MappedPageTable, MappedPageTableMut, PageTable, PageTableIndex,
    PageTableLevel, Result, L1, L4,
};
use crate::physmem::{self, Frame};
use core::mem::ManuallyDrop;
use core::ops::{Deref, DerefMut};

#[must_use = "Must use a mapper flush"]
pub struct MapperFlush(u64);

impl MapperFlush {
    pub fn new(addr: u64) -> Self {
        Self(addr)
    }

    pub fn flush(self, active: &ActivePageTable) {
        let mdself = ManuallyDrop::new(self);
        active.flush(mdself.0);
    }

    pub unsafe fn ignore(self) {
        let _ = ManuallyDrop::new(self);
    }
}

impl Drop for MapperFlush {
    fn drop(&mut self) {
        panic!("Ignored a mapper flush");
    }
}

#[must_use = "Must use a mapper flush"]
pub struct MapperFlushAll(bool);

impl MapperFlushAll {
    pub fn new() -> Self {
        Self(false)
    }

    pub fn consume(&mut self, flush: MapperFlush) {
        let _ = ManuallyDrop::new(flush);
        self.0 = true;
    }

    pub fn flush(self, active: &ActivePageTable) {
        if self.0 {
            let _ = ManuallyDrop::new(self);
            active.flush_all();
        }
    }

    pub unsafe fn ignore(self) {
        let _ = ManuallyDrop::new(self);
    }
}

impl Drop for MapperFlushAll {
    fn drop(&mut self) {
        assert!(!self.0, "Ignored a mapper flush all");
    }
}

pub struct MappedPteReference<L: PageTableLevel> {
    page_table: MappedPageTable<L>,
    index: PageTableIndex,
}

impl<L: PageTableLevel> Deref for MappedPteReference<L> {
    type Target = RawPte;
    fn deref(&self) -> &Self::Target {
        &self.page_table[self.index]
    }
}

pub struct MappedMutPteReference<L: PageTableLevel> {
    page_table: MappedPageTableMut<L>,
    index: PageTableIndex,
}

impl<'a, L: PageTableLevel> Deref for MappedMutPteReference<L> {
    type Target = RawPte;
    fn deref(&self) -> &Self::Target {
        &self.page_table[self.index]
    }
}

impl<'a, L: PageTableLevel> DerefMut for MappedMutPteReference<L> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.page_table[self.index]
    }
}

pub struct Mapper {
    p4_mapping: HyperspaceMapping,
}

impl Mapper {
    pub unsafe fn new(p4_frame: Frame) -> Result<Self> {
        map_page(p4_frame).map(|p4_mapping| Self { p4_mapping })
    }

    pub fn p4(&self) -> &PageTable<L4> {
        unsafe { &*self.p4_mapping.as_ptr() }
    }

    pub fn p4_mut(&mut self) -> &mut PageTable<L4> {
        unsafe { &mut *self.p4_mapping.as_mut_ptr() }
    }

    pub fn get_pte_for_address(&self, addr: u64) -> Result<MappedPteReference<L1>> {
        self.p4()
            .next_table(p4_index(addr))
            .and_then(|p3| p3.next_table(p3_index(addr)))
            .and_then(|p2| p2.next_table(p2_index(addr)))
            .map(|p1| MappedPteReference {
                page_table: p1,
                index: p1_index(addr),
            })
    }

    pub fn get_pte_mut_for_address(&mut self, addr: u64) -> Result<MappedMutPteReference<L1>> {
        self.p4_mut()
            .next_table_mut(p4_index(addr))
            .and_then(|mut p3| p3.next_table_mut(p3_index(addr)))
            .and_then(|mut p2| p2.next_table_mut(p2_index(addr)))
            .map(|p1| MappedMutPteReference {
                page_table: p1,
                index: p1_index(addr),
            })
    }

    pub fn create_pte_mut_for_address(&mut self, addr: u64) -> Result<MappedMutPteReference<L1>> {
        let mut p1 = self
            .p4_mut()
            .create_next_table(p4_index(addr))?
            .create_next_table(p3_index(addr))?
            .create_next_table(p2_index(addr))?;

        Ok(MappedMutPteReference {
            page_table: p1,
            index: p1_index(addr),
        })
    }

    pub fn map_to(
        &mut self,
        page: u64,
        frame: Frame,
        flags: PresentPageFlags,
    ) -> Result<MapperFlush> {
        let mut pte = self.create_pte_mut_for_address(page)?;

        assert_eq!(*pte, RawPte::unused());
        assert!(pte.is_unused());
        *pte = RawPresentPte::from_frame_and_flags(frame, flags).into();
        Ok(MapperFlush::new(page))
    }

    pub fn unmap_and_free(&mut self, page: u64) -> Result<MapperFlush> {
        // We can improve this - particularly we can avoid all of the flushing
        // and not create page tables. Also, we should be able to delete page tables if they're no longer needed
        let mut pte = self.create_pte_mut_for_address(page)?;

        if pte.is_present() {
            physmem::deallocate_frame(pte.present().unwrap().frame());
        }

        *pte = RawPte::unused();
        Ok(MapperFlush::new(page))
    }

    pub fn set_not_present(
        &mut self,
        page: u64,
        npp: impl Into<RawNotPresentPte>,
    ) -> Result<MapperFlush> {
        let mut pte = self.create_pte_mut_for_address(page)?;

        // We should only be doing this for unused pages
        assert_eq!(*pte, RawPte::unused());
        assert!(pte.is_unused());
        *pte = npp.into().into();
        Ok(MapperFlush::new(page))
    }
}
