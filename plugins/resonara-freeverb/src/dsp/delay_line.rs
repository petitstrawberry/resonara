//! Fixed-capacity storage replaces Freeverb's Vec; no OS allocator imports.
//! Maximum delay at 96 kHz is 3570 samples, including the stereo spread.
const MAX_DELAY: usize = 4096;
pub struct DelayLine {
    buffer: [f64; MAX_DELAY],
    index: usize,
    length: usize,
}
impl DelayLine {
    pub const fn new(length: usize) -> Self {
        Self {
            buffer: [0.0; MAX_DELAY],
            index: 0,
            length,
        }
    }
    pub fn configure(&mut self, length: usize) {
        assert!(length > 0 && length <= MAX_DELAY);
        self.length = length;
        self.clear();
    }
    pub fn clear(&mut self) {
        self.buffer[..self.length].fill(0.0);
        self.index = 0;
    }
    pub fn read(&self) -> f64 {
        self.buffer[self.index]
    }
    pub fn write_and_advance(&mut self, value: f64) {
        self.buffer[self.index] = value;
        self.index = if self.index == self.length - 1 {
            0
        } else {
            self.index + 1
        };
    }
}
