use std::collections::VecDeque;

use macroquad::input;

use crate::{ types::{ Player, PlayerID, PlayerInput }, MAX_PLAYER_COUNT };

#[derive(Debug, Clone)]
pub struct PlayerInputs {
    pub inputs: [Option<Vec<PlayerInput>>; MAX_PLAYER_COUNT as usize],
    pub frame: u32,
}

impl PlayerInputs {
    fn new(frame: u32) -> Self {
        PlayerInputs {
            inputs: [None, None],
            frame,
        }
    }

    fn insert_player_input(&mut self, input: Vec<PlayerInput>, player_id: PlayerID) {
        // println!("input before {:?}", self.inputs);
        self.inputs[player_id as usize] = Some(input);
        // println!("input before {:?}", self.inputs);
    }

    pub fn is_verified(&self, player_count: u8) -> bool {
        let amt = self.inputs
            .iter()
            .enumerate()
            .filter_map(|(idx, i)| {
                if idx < (player_count as usize) && i.is_some() { Some(i) } else { None }
            })
            .count();
        amt == (player_count as usize)
    }
}

#[derive(Debug)]
pub struct InputBuffer {
    pub input_frames: VecDeque<PlayerInputs>,
    last_verified_inputs: [Option<Vec<PlayerInput>>; MAX_PLAYER_COUNT as usize],
    pub player_count: u8,
    local_player: PlayerID,
}

impl InputBuffer {
    pub fn new() -> Self {
        InputBuffer {
            input_frames: VecDeque::new(),
            last_verified_inputs: [None, None],
            player_count: 1,
            local_player: PlayerID::Player1,
        }
    }
    pub fn update_player_count(
        &mut self,
        local_player: PlayerID,
        player_cnt: u8,
        curr_verified_frame: u32
    ) {
        if local_player == self.local_player {
            // verified sim is running in single player so when it switches then we need to reset this
            self.last_verified_inputs = [None, None];
        } else {
            // other player accumulates until world data is sent over, we can and have to remove all these to continue
            self.input_frames.retain(|input_frame| input_frame.frame >= curr_verified_frame + 1);
            // swap because we changed player we need to swap inputs to new location
            self.input_frames
                .iter_mut()
                .for_each(|input_frame|
                    input_frame.inputs.swap(local_player as usize, self.local_player as usize)
                );
        }
        self.local_player = local_player;
        println!("updating player count to {:?}", self);
        self.player_count = player_cnt;
    }
    pub fn insert_curr_player_inp(&mut self, inp: Vec<PlayerInput>, frame: u32) {
        debug_assert!(frame != 0); // no input can happen before its first drawn
        // frame 0 doesnt exist in arra
        // println!(
        //     "inserted curr player {:?} input at frame {}, input {:?}",
        //     self.local_player,
        //     frame,
        //     inp
        // );
        while self.input_frames.back().map_or(0, |pi| pi.frame) < frame {
            let next_frame = self.input_frames.back().map_or(frame, |pi| pi.frame + 1);
            let new_inp = PlayerInputs::new(next_frame);
            self.input_frames.push_back(new_inp);
            // println!("inserting frame local {:?} for frame {:?}", inp, frame);
        }
        if let Some(existing_input) = self.input_frames.iter_mut().find(|pi| pi.frame == frame) {
            existing_input.insert_player_input(inp, self.local_player);
            // println!("new existing input {:?}", existing_input);
        } else {
            let mut new_inputs = PlayerInputs::new(frame);
            new_inputs.insert_player_input(inp, self.local_player);
            self.input_frames.insert(
                self.input_frames.partition_point(|pi| pi.frame < frame),
                new_inputs
            );
        }

        // debug_assert!(
        //     self.input_frames
        //         .iter()
        //         .take_while(|pi| pi.frame < frame && pi.frame < self.curr_verified_frame)
        //         .all(|pi| pi.inputs[self.local_player as usize].is_some()),
        //     "Missing input for local player in a frame before the current one || Local player input has to be contigious inp_frame: {:?}",
        //     self.input_frames.iter().find(|inp| inp.inputs[self.local_player as usize].is_none())
        // );

        debug_assert!(
            self.input_frames
                .iter()
                .zip(self.input_frames.iter().skip(1))
                .all(|(a, b)| a.frame <= b.frame)
        );
        // println!(
        //     "state after inserting curr player now {:?}",
        //     self.input_frames.iter().find(|f| f.frame == frame)
        // );
    }
    pub fn insert_other_player_inp(&mut self, inp: Vec<PlayerInput>, frame: u32) {
        if
            let Some(first_input_frame_local) = self.input_frames
                .iter()
                .find(|input_frame| input_frame.inputs[self.local_player as usize].is_some())
        {
            if frame < first_input_frame_local.frame {
                println!(
                    "tried to insert frame thats before current own player input frame : {}, actual curr frame ; {}",
                    frame,
                    first_input_frame_local.frame
                );
                return;
            }
        }

        debug_assert!(frame != 0); // no input can happen before its first drawn
        // debug_assert!(other != self.local_player);
        // frame 0 doesnt exist in arra
        let other_player_id = if self.local_player == PlayerID::Player1 {
            PlayerID::Player2
        } else {
            PlayerID::Player1
        };
        // println!(
        //     "inserted other player {:?} input at frame {}, input {:?}",
        //     other_player_id,
        //     frame,
        //     inp
        // );
        while self.input_frames.back().map_or(0, |pi| pi.frame) < frame {
            let next_frame = self.input_frames.back().map_or(frame, |pi| pi.frame + 1);
            let inp = PlayerInputs::new(next_frame);
            self.input_frames.push_back(inp);
        }
        if let Some(existing_input) = self.input_frames.iter_mut().find(|pi| pi.frame == frame) {
            existing_input.insert_player_input(inp, other_player_id);
            // println!("updated existing input with new inp {:?}", existing_input);
        } else {
            let mut new_inputs = PlayerInputs::new(frame);
            new_inputs.insert_player_input(inp, other_player_id);
            self.input_frames.insert(
                self.input_frames.partition_point(|pi| pi.frame < frame),
                new_inputs
            );
        }
        debug_assert!(
            self.input_frames
                .iter()
                .zip(self.input_frames.iter().skip(1))
                .all(|(a, b)| a.frame <= b.frame)
        );
        // println!(
        //     "state after inserting other now {:?}",
        //     self.input_frames.iter().find(|f| f.frame == frame)
        // );
    }
    pub fn pop_next_verified_frame(&mut self) -> Option<PlayerInputs> {
        if let Some(front) = self.input_frames.front() {
            if front.is_verified(self.player_count) {
                let res = self.input_frames.pop_front().unwrap();
                self.last_verified_inputs = res.inputs.clone();

                return Some(res);
            }
        }
        None
    }

    pub fn excluding_iter_after_last_verified(
        &self
    ) -> impl Iterator<Item = (usize, PlayerInputs)> + '_ {
        (0..self.input_frames.len()).filter_map(|index| {
            let frame_input = &self.input_frames[index];
            let mut new_input = frame_input.clone();

            // BELOW SHOULD BE HANDLED BY the loop that calls this, by checking for frame

            // if new_input.is_verified(self.player_count) {
            //     // TODO VERIFY WE DONT NEED THIS
            //     // if our input is None it means the other sim is ahead of us and we can skip this for now ||
            //     //new_input.inputs[self.local_player as usize].is_none()
            //     println!("frame num of verified{}", new_input.frame);
            //     return None;
            // }
            //println!("input is verified {}", new_input.is_verified(self.player_count));
            for (player_id, input) in new_input.inputs.iter_mut().enumerate() {
                if input.is_some() {
                    continue;
                }
                if self.last_verified_inputs[0].is_some() && self.last_verified_inputs[1].is_some() {
                    *input = self.last_verified_inputs[player_id].clone();
                }
            }
            Some((index, new_input))
        })
    }
}
