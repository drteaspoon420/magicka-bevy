//! SubMenuIntro's fresh-campaign sequence. The host owns tome rendering,
//! subtitles, cues and transition callbacks; update/draw ordering is retained.

pub const PAGE_SECONDS: [f32; 8] = [17.0, 6.0, 20.0, 11.0, 17.0, 13.0, 9.0, 18.0];

#[derive(Clone, Debug, PartialEq)]
pub enum IntroEvent {
    StartMusic,
    /// Tome light intensity, variation amount and variation speed.
    Light([f32; 3]),
    PlayNarration(usize),
    StopNarration,
    ShowOpeningSubtitles,
    HideSubtitles,
    /// Wrap this segment to 896 pixels and fill the back text buffer.
    PreparePage(usize),
    Riffle,
    FadeToBlack {
        seconds: f32,
    },
    /// Final narration becomes the loading tip unless the user skipped.
    LoadFirstLevel {
        loading_tip: Option<usize>,
    },
}

#[derive(Clone, Debug)]
pub struct IntroPlayback {
    /// None during the dark opening; then chapter 0 and illustrations 1..6.
    /// Segment 7 is narration handed to the loading screen.
    pub index: Option<usize>,
    pub remaining: f32,
    pub texture_index: usize,
    pub swap_pending: bool,
    cue_present: bool,
    finish: bool,
    finishing: bool,
    fading: bool,
    complete: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct IntroPage {
    pub index: usize,
    pub old_index: isize,
    /// Double SmoothStep of the middle 80% of this page's elapsed time.
    pub motion: f32,
    /// The host swaps the retained text buffers only on a new-page draw.
    pub swap_text: bool,
}

impl IntroPlayback {
    pub fn enter() -> (Self, Vec<IntroEvent>) {
        (
            Self {
                index: None,
                remaining: 8.0,
                texture_index: 0,
                swap_pending: false,
                cue_present: false,
                finish: false,
                finishing: false,
                fading: false,
                complete: false,
            },
            vec![IntroEvent::StartMusic, IntroEvent::Light([0.0, 0.0, 1.5])],
        )
    }

    pub fn narration_name(index: usize) -> String {
        format!("cuts_int{index}")
    }
    pub fn text_key(index: usize) -> String {
        format!("#cutscene_intro{index}")
    }

    /// Controller A skips with the tome open. Mouse skipping additionally
    /// requires the previous button to have been pressed (a release edge).
    pub fn skip(&mut self, tome_open: bool) -> Vec<IntroEvent> {
        if !tome_open || self.complete {
            return Vec::new();
        }
        self.finish = true;
        if self.cue_present {
            vec![IntroEvent::StopNarration]
        } else {
            Vec::new()
        }
    }

    pub fn mouse_action(
        &mut self,
        tome_open: bool,
        pressed: bool,
        old_pressed: bool,
    ) -> Vec<IntroEvent> {
        if !pressed && old_pressed {
            self.skip(tome_open)
        } else {
            Vec::new()
        }
    }

    /// Cue presence is retained separately from whether it is still playing:
    /// the intro advances by authored timers, not by narration completion.
    pub fn update(&mut self, dt: f32, tome_open: bool) -> Vec<IntroEvent> {
        use IntroEvent::*;
        if self.complete {
            return Vec::new();
        }
        self.remaining = (self.remaining - dt).max(0.0);
        let mut events = Vec::new();
        if tome_open && (self.index.is_none() || !self.cue_present) {
            match self.index {
                None if self.remaining <= 0.0 => {
                    self.index = Some(0);
                    self.remaining = PAGE_SECONDS[0];
                    events.push(Light([1.0, 0.2, 0.5]));
                }
                Some(index) => {
                    self.cue_present = true;
                    events.push(PlayNarration(index));
                }
                None => {}
            }
        }
        if self.finish {
            if !self.finishing {
                self.finishing = true;
                self.fading = true;
                events.push(FadeToBlack { seconds: 2.0 });
            }
        } else if let Some(index) = self.index {
            if self.remaining <= 0.0 {
                events.push(HideSubtitles);
                if index < 7 {
                    let next = index + 1;
                    if next == 7 {
                        self.fading = true;
                        events.push(FadeToBlack { seconds: 2.0 });
                    } else {
                        events.push(PreparePage(next));
                        self.cue_present = false;
                        self.remaining = PAGE_SECONDS[next];
                        self.swap_pending = true;
                        events.push(Riffle);
                    }
                    self.index = Some(next);
                }
            }
        } else if self.remaining <= 5.0 && !self.cue_present {
            self.cue_present = true;
            events.extend([PlayNarration(0), ShowOpeningSubtitles]);
        }
        events
    }

    /// SubMenuIntro.Draw: text-buffer swap is a draw-side operation. DrawOld
    /// reads texture_index-1 after that swap and uses motion=1.
    pub fn draw_page(&mut self) -> IntroPage {
        let swap_text = self.swap_pending;
        if swap_text {
            self.texture_index = self.index.expect("a pending page has an index");
            self.swap_pending = false;
        }
        let time = ((1.0 - self.remaining / PAGE_SECONDS[self.texture_index]) / 0.8 - 0.125)
            .clamp(0.0, 1.0);
        let smooth = |t: f32| t * t * (3.0 - 2.0 * t);
        IntroPage {
            index: self.texture_index,
            old_index: self.texture_index as isize - 1,
            motion: smooth(smooth(time)),
            swap_text,
        }
    }

    pub fn transition_end(&mut self) -> Option<IntroEvent> {
        if !self.fading || self.complete {
            return None;
        }
        self.complete = true;
        Some(IntroEvent::LoadFirstLevel {
            loading_tip: if self.finish { None } else { self.index },
        })
    }

    pub fn exit_events() -> [IntroEvent; 2] {
        [
            IntroEvent::Light([1.0, 0.2, 6.0]),
            IntroEvent::HideSubtitles,
        ]
    }
}

/// DrawImagePan1/Zoom1/Zoom2's shader UV offset and scale. Illustration
/// indices 3/4/5 move; 1/2/6 are static. The mask quad is 852 by 652,
/// centered on (512,416) in a 1024-square right-page viewport.
pub fn illustration_uv(index: usize, time: f32, texture_size: [f32; 2]) -> ([f32; 2], [f32; 2]) {
    let [width, height] = texture_size;
    match index {
        3 => ([(100.0 + time * 600.0) / width, 0.0], [1.0; 2]),
        4 => {
            let t = 1.0 - time;
            ([t * 0.25, t * 64.0 / height], [2.0 - t; 2])
        }
        5 => (
            [time * 280.0 / width, time * 220.0 / height],
            [1.0 + time * 0.1; 2],
        ),
        _ => ([0.0; 2], [1.0; 2]),
    }
}
