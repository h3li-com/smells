pub struct Scalars {
    pub a: i32,
    pub b: i32,
    pub c: i32,
    pub d: i32,
    pub e: i32,
}

pub struct PositionA { pub x: i32, pub y: i32, pub z: i32 }
pub struct PositionB { pub x: i32, pub y: i32, pub z: i32 }
pub struct PositionC { pub x: i32, pub y: i32, pub z: i32 }

pub struct First;
impl First {
    pub fn alpha(&self, x: i32) -> i32 {
        let y = x + 2;
        let z = y * 3;
        let w = z - 5;
        w + 4
    }
}
pub struct Second;
impl Second {
    pub fn beta(&self, x: i32) -> i32 {
        let y = x + 9;
        let z = y * 7;
        let w = z - 6;
        w + 8
    }
}

pub struct OptionalState {
    pub first: Option<i32>,
    pub second: Option<i32>,
    pub third: Option<i32>,
}
impl OptionalState {
    pub fn a(&self) {}
    pub fn b(&self) {}
    pub fn c(&self) {}
    pub fn read(&self) {
        let _ = self.first;
        let _ = self.second;
        let _ = self.third;
    }
}

pub struct Backend;
impl Backend {
    pub fn a(&self, x: i32) -> i32 { x }
    pub fn b(&self, x: i32) -> i32 { x }
    pub fn c(&self, x: i32) -> i32 { x }
    pub fn d(&self, x: i32) -> i32 { x }
    pub fn e(&self, x: i32) -> i32 { x }
}
pub struct Forwarder { pub backend: Backend }
impl Forwarder {
    pub fn a(&self, x: i32) -> i32 { self.backend.a(x) }
    pub fn b(&self, x: i32) -> i32 { self.backend.b(x) }
    pub fn c(&self, x: i32) -> i32 { self.backend.c(x) }
    pub fn d(&self, x: i32) -> i32 { self.backend.d(x) }
    pub fn e(&self, x: i32) -> i32 { self.backend.e(x) }
}

pub struct Marker;
pub trait Port { fn send(&self); }
pub enum State { A, B, C, D }
pub fn dispatch_a(state: State) -> i32 {
    match state { State::A => 1, State::B => 2, State::C => 3, State::D => 4 }
}
pub fn dispatch_b(state: State) -> i32 {
    match state { State::A => 5, State::B => 6, State::C => 7, State::D => 8 }
}
pub fn dispatch_c(state: State) -> i32 {
    match state { State::A => 9, State::B => 10, State::C => 11, State::D => 12 }
}

pub fn explained() {
    // Why comment 1.
    // Why comment 2.
    // Why comment 3.
    // Why comment 4.
    // Why comment 5.
    // Why comment 6.
    // Why comment 7.
    // Why comment 8.
    // Why comment 9.
    let _ = 1;
    let _ = 2;
    let _ = 3;
    let _ = 4;
    let _ = 5;
    let _ = 6;
    let _ = 7;
    let _ = 8;
    let _ = 9;
    let _ = 10;
    let _ = 11;
    let _ = 12;
    let _ = 13;
    let _ = 14;
    let _ = 15;
    let _ = 16;
    let _ = 17;
    let _ = 18;
    let _ = 19;
    let _ = 20;
    let _ = 21;
}
