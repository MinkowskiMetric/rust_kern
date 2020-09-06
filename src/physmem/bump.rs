use crate::physmem::{Frame, FrameAllocator, MemoryArea, MemoryMapIterator, PAGE_SIZE};

pub struct BumpAllocator {
    areas: MemoryMapIterator,
    current_area: Option<&'static MemoryArea>,
    used_frames: usize,
    next_free_frame: u64,
}

impl BumpAllocator {
    pub fn new(areas: MemoryMapIterator) -> Self {
        Self {
            areas,
            current_area: None,
            used_frames: 0,
            next_free_frame: 0,
        }
    }
}

impl FrameAllocator for BumpAllocator {
    fn used_frames(&self) -> usize {
        self.used_frames
    }

    fn free_frames(&self) -> usize {
        let mut count = if let Some(ref current_area) = self.current_area {
            assert!(
                self.next_free_frame >= current_area.start
                    && self.next_free_frame <= current_area.limit
            );
            (current_area.limit - self.next_free_frame) / PAGE_SIZE
        } else {
            0
        };

        for area in self.areas.clone() {
            assert!(self.next_free_frame < area.start || self.next_free_frame >= area.limit);
            count += (area.limit - area.start) / PAGE_SIZE;
        }

        count as usize
    }

    fn allocate_frame(&mut self) -> Option<Frame> {
        loop {
            if let Some(ref current_area) = self.current_area {
                assert!(
                    self.next_free_frame >= current_area.start
                        && self.next_free_frame <= current_area.limit
                );
                if self.next_free_frame < current_area.limit {
                    let ret = self.next_free_frame;
                    self.next_free_frame += PAGE_SIZE;
                    self.used_frames += 1;
                    return Some(Frame::containing_address(ret));
                } else {
                    // Otherwise, we've exhausted this region completely so we need to move on
                    self.current_area = None;
                }
            }

            if let Some(ref next_area) = self.areas.next() {
                self.current_area = Some(next_area);
                self.next_free_frame = next_area.start;
            } else {
                return None;
            }
        }
    }

    fn deallocate_frame(&mut self, _frame: Frame) {
        panic!("BumpAllocator cannot deallocate frames");
    }
}