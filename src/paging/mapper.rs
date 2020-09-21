use super::page_entry::{PresentPageFlags, RawNotPresentPte, RawPresentPte, RawPte};
use super::{
    p1_index, p2_index, p3_index, p4_index, phys_to_virt_mut, ActivePageTable, PageTable, Result,
    L4,
};
use crate::physmem::{self, Frame};
use core::mem::ManuallyDrop;

#[must_use = "Must use a mapper flush"]
pub struct MapperFlush(usize);

impl MapperFlush {
    pub fn new(addr: usize) -> Self {
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

pub struct Mapper {
    p4: &'static mut PageTable<L4>,
}

impl Mapper {
    pub unsafe fn new(p4_frame: Frame) -> Self {
        Self {
            p4: &mut *phys_to_virt_mut(p4_frame.physical_address()),
        }
    }

    pub fn p4(&self) -> &PageTable<L4> {
        &self.p4
    }

    pub fn p4_mut(&mut self) -> &mut PageTable<L4> {
        &mut self.p4
    }

    pub fn get_pte_for_address<'a>(&'a self, addr: usize) -> Option<&'a RawPte> {
        self.p4()
            .next_table(p4_index(addr))
            .and_then(|p3| p3.next_table(p3_index(addr)))
            .and_then(|p2| p2.next_table(p2_index(addr)))
            .map(|p1| &p1[p1_index(addr)])
    }

    pub fn get_pte_mut_for_address<'a>(&'a mut self, addr: usize) -> Option<&'a mut RawPte> {
        self.p4_mut()
            .next_table_mut(p4_index(addr))
            .and_then(|p3| p3.next_table_mut(p3_index(addr)))
            .and_then(|p2| p2.next_table_mut(p2_index(addr)))
            .map(|p1| &mut p1[p1_index(addr)])
    }

    pub fn create_pte_mut_for_address<'a>(&'a mut self, addr: usize) -> Result<&'a mut RawPte> {
        let p1 = self
            .p4_mut()
            .create_next_table(p4_index(addr))?
            .create_next_table(p3_index(addr))?
            .create_next_table(p2_index(addr))?;

        Ok(&mut p1[p1_index(addr)])
    }

    pub fn map_to(
        &mut self,
        page: usize,
        frame: Frame,
        flags: PresentPageFlags,
    ) -> Result<MapperFlush> {
        let pte = self.create_pte_mut_for_address(page)?;

        assert_eq!(*pte, RawPte::unused());
        assert!(pte.is_unused());
        *pte = RawPresentPte::from_frame_and_flags(frame, flags).into();
        Ok(MapperFlush::new(page))
    }

    pub fn unmap_and_free(&mut self, page: usize) -> MapperFlush {
        self.unmap_and_free_and_replace(page, RawNotPresentPte::unused())
    }

    pub fn unmap_and_free_and_replace(
        &mut self,
        page: usize,
        new_pte: impl Into<RawNotPresentPte>,
    ) -> MapperFlush {
        // We can improve this - particularly we can avoid all of the flushing
        // and not create page tables. Also, we should be able to delete page tables if they're no longer needed
        let pte = self
            .get_pte_mut_for_address(page)
            .filter(|pte| pte.is_present())
            .expect("Unmapping page which is not mapped");

        physmem::deallocate_frame(pte.present().unwrap().frame());
        *pte = new_pte.into().into();
        MapperFlush::new(page)
    }

    pub fn set_present(
        &mut self,
        page: usize,
        new_pte: impl Into<RawPresentPte>,
    ) -> Result<MapperFlush> {
        self.do_set_pte(page, new_pte.into())
    }

    pub fn set_not_present(
        &mut self,
        page: usize,
        new_pte: impl Into<RawNotPresentPte>,
    ) -> Result<MapperFlush> {
        self.do_set_pte(page, new_pte.into())
    }

    fn do_set_pte(&mut self, page: usize, new_pte: impl Into<RawPte>) -> Result<MapperFlush> {
        let pte = self.create_pte_mut_for_address(page)?;

        // We should only be doing this for not present pages
        assert!(!pte.is_present());
        *pte = new_pte.into();
        Ok(MapperFlush::new(page))
    }
}
