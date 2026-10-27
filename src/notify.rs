pub struct Handle;

impl Handle {
    pub fn closed(&self, _id: u32, _reason: u32) {}

    pub fn clicked(&self, _id: u32) {}
}
