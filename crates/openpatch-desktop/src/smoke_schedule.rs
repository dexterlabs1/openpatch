//! Capture-only scheduling for ten already-static smoke scenes.
//! Keep setup, two warm draws, the original request, and a saved receipt before
//! changing scenes. Interactive repaint and every animation/input fixture use
//! their existing clocks.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Receipt {
    pub frame: u32,
    pub capture_index: usize,
}

impl Receipt {
    pub(crate) fn accepts(self, token: Option<Self>, capture_index: usize) -> bool {
        token == Some(self) && self.capture_index == capture_index
    }
}

fn scene_start(frame: u32) -> Option<u32> {
    (1350..=1826)
        .contains(&frame)
        .then(|| 1350 + (frame - 1350) / 50 * 50)
}

pub(crate) fn request(frame: u32, capture_index: usize) -> Option<Receipt> {
    let start = scene_start(frame)?;
    (frame == start + 22).then_some(Receipt {
        frame,
        capture_index,
    })
}

pub(crate) fn next_frame(frame: u32, pending: Option<Receipt>) -> u32 {
    if let Some(start) = scene_start(frame) {
        if frame == start + 4 {
            return start + 22;
        }
        if frame == start + 24 {
            return if pending.is_some() {
                frame
            } else if start == 1800 {
                1828
            } else {
                start + 50
            };
        }
    }
    if frame == 0 {
        1
    } else {
        (frame / 2 + 1) * 2
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn static_smoke_scenes_keep_warm_draws_requests_and_delayed_saved_receipts() {
        for delay in [0, 1, 8, 100] {
            let mut frame = 0;
            let mut pending = None;
            let mut waiting = 0;
            let mut requested = Vec::new();
            let mut visited = BTreeSet::new();
            let mut saved = 0;
            for _ in 0..5000 {
                frame = next_frame(frame, pending);
                visited.insert(frame);
                if let Some(receipt) = request(frame, saved) {
                    assert!(pending.is_none(), "Never repeat a request while waiting");
                    pending = Some(receipt);
                    waiting = delay;
                    requested.push(frame);
                } else if let Some(receipt) = pending {
                    assert_eq!(frame, receipt.frame + 2);
                    // A foreign token cannot satisfy the pending receipt.
                    assert!(!receipt.accepts(
                        Some(Receipt {
                            frame: receipt.frame - 2,
                            ..receipt
                        }),
                        saved
                    ));
                    assert!(!receipt.accepts(None, saved));
                    assert!(!receipt.accepts(Some(receipt), saved + 1));
                    assert!(receipt.accepts(Some(receipt), saved));
                    assert_eq!(receipt.capture_index, saved);
                    if waiting == 0 {
                        pending = None; // caller validated and saved this exact receipt
                        saved += 1;
                    } else {
                        waiting -= 1;
                    }
                }
                if frame >= 2760 {
                    break;
                }
            }
            assert_eq!(saved, 10);
            assert_eq!(
                requested,
                (1350..=1800)
                    .step_by(50)
                    .map(|s| s + 22)
                    .collect::<Vec<_>>()
            );
            for start in (1350..=1800).step_by(50) {
                for offset in [0, 2, 4, 22, 24] {
                    assert!(visited.contains(&(start + offset)));
                }
            }
            // All other fixture/input/animation milestones are unchanged.
            for frame in (2..=2760).step_by(2).filter(|f| !(1350..=1826).contains(f)) {
                assert!(visited.contains(&frame), "Lost original frame {frame}");
            }
        }
    }
}
