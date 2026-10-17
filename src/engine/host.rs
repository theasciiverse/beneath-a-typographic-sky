//! What the engine needs from the world around it: time, input, and a place
//! to show its frames. Stands in for ScummVM's OSystem. The terminal front
//! end implements it; tests use a headless one.

/// The keyboard shortcuts of ScummVM's sky keymap.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    None,
    ToggleFastMode,
    ToggleReallyFastMode,
    OpenControlPanel,
    Skip,
    SkipLine,
    Pause,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostEvent {
    Action(Action),
    /// any other key
    Key,
    /// pointer moved, in game pixels (0..320, 0..200)
    MouseMove(u16, u16),
    /// button pressed: 2 is left (look), 1 is right (use), as ScummVM maps them
    Button { x: u16, y: u16, button: u8 },
    ButtonUp,
    Quit,
}

/// One text sprite as the engine drew it, so the front end can draw real
/// characters over the pixels.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TextOverlay {
    /// top left in screen pixels; may be partly off screen
    pub x: i32,
    pub y: i32,
    pub width: u16,
    pub line_height: u16,
    pub lines: Vec<String>,
    /// palette index of the text colour
    pub color: u8,
    /// the colour itself, filled in by the front end
    pub color_rgb: [u8; 3],
    pub centered: bool,
    /// Some(height in pixels): a pixel button, drawn as a framed box this
    /// tall with the label in it
    pub boxed: Option<u16>,
}

/// The mouse pointer as a small indexed sprite.
#[derive(Debug, Clone, Default)]
pub struct Cursor {
    pub visible: bool,
    pub width: u16,
    pub height: u16,
    pub hot_x: u16,
    pub hot_y: u16,
    pub pixels: Vec<u8>,
    /// which pointer it is (MOUSE_NORMAL, MOUSE_CROSS, an object ...)
    pub kind: u16,
}

pub struct Frame<'a> {
    /// 320x200 palette indices
    pub pixels: &'a [u8],
    /// 256 RGB entries, 0..255
    pub palette: &'a [u8; 768],
    pub texts: &'a [TextOverlay],
    pub cursor: &'a Cursor,
    pub mouse_x: u16,
    pub mouse_y: u16,
}

/// One line of a menu the host shows (the options panel, save slots).
#[derive(Debug, Clone)]
pub struct MenuItem {
    pub label: String,
    pub enabled: bool,
}

impl MenuItem {
    pub fn new(label: &str) -> MenuItem {
        MenuItem { label: label.to_string(), enabled: true }
    }

    pub fn enabled(mut self, on: bool) -> MenuItem {
        self.enabled = on;
        self
    }
}

pub trait Host {
    fn millis(&self) -> u64;
    fn sleep(&mut self, ms: u32);
    fn poll(&mut self) -> Vec<HostEvent>;
    /// Called whenever ScummVM would call updateScreen.
    fn present(&mut self, frame: &Frame);
    /// Is a mouse button held down right now?
    fn button_held(&self) -> bool {
        false
    }
    /// Should the engine open the sound device?
    fn wants_audio(&self) -> bool {
        false
    }
    /// A modal menu; the chosen index, or None when dismissed.
    fn menu(&mut self, _title: &str, _items: &[MenuItem]) -> Option<usize> {
        None
    }
    fn confirm(&mut self, _question: &str) -> bool {
        false
    }
    fn message(&mut self, text: &str) {
        log::info!("{text}");
    }
    fn text_input(&mut self, _prompt: &str, default: &str) -> Option<String> {
        Some(default.to_string())
    }
}

/// A host with no screen and a fake clock, for tests and tools.
pub struct Headless {
    pub now: u64,
    pub events: std::collections::VecDeque<HostEvent>,
    pub frames: u64,
    pub last: Vec<u8>,
    pub last_palette: Vec<u8>,
    pub last_texts: Vec<TextOverlay>,
}

impl Headless {
    pub fn new() -> Headless {
        Headless {
            now: 0,
            events: Default::default(),
            frames: 0,
            last: Vec::new(),
            last_palette: Vec::new(),
            last_texts: Vec::new(),
        }
    }
}

impl Host for Headless {
    fn millis(&self) -> u64 {
        self.now
    }
    fn sleep(&mut self, ms: u32) {
        self.now += ms as u64;
    }
    fn poll(&mut self) -> Vec<HostEvent> {
        // one event per poll, so scripted input spreads over game cycles
        self.events.pop_front().into_iter().collect()
    }
    fn present(&mut self, frame: &Frame) {
        self.frames += 1;
        self.last.clear();
        self.last.extend_from_slice(frame.pixels);
        self.last_palette.clear();
        self.last_palette.extend_from_slice(frame.palette);
        self.last_texts = frame.texts.to_vec();
    }
}
