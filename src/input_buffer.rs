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
        self.inputs[player_id as usize] = Some(input);
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
    pub inputs: VecDeque<PlayerInputs>,
    last_verified_inputs: [Option<Vec<PlayerInput>>; MAX_PLAYER_COUNT as usize],
    pub player_count: u8,
    curr_frame: u32,
    local_player: PlayerID,
}

impl InputBuffer {
    pub fn new() -> Self {
        InputBuffer {
            inputs: VecDeque::new(),
            last_verified_inputs: [None, None],
            player_count: 1,
            curr_frame: 0,
            local_player: PlayerID::Player1,
        }
    }
    pub fn update_player_count(&mut self, sim_frame: u32, local_player: PlayerID, player_cnt: u8) {
        for input in &mut self.inputs {
            input.inputs = [None, None];
        }
        self.player_count = player_cnt;
        self.local_player = local_player;
        self.curr_frame = sim_frame;
        self.last_verified_inputs = [None, None];
    }
    pub fn insert_curr_player_inp(&mut self, inp: Vec<PlayerInput>, frame: u32) {
        debug_assert!(frame != 0); // no input can happen before its first drawn
        // frame 0 doesnt exist in arra
        for i in self.curr_frame + 1..=frame {
            self.inputs.push_back(PlayerInputs::new(i));
        }
        let last_idx = self.inputs.len() - 1;
        self.inputs[last_idx].insert_player_input(inp, self.local_player);
        self.curr_frame = frame;

        // debug_assert!(
        //     self.inputs
        //         .iter()
        //         .zip(self.inputs.iter().skip(1))
        //         .all(|(a, b)| a.frame <= b.frame)
        // );
    }
    pub fn insert_other_player_inp(&mut self, inp: Vec<PlayerInput>, frame: u32) {
        debug_assert!(frame != 0); // no input can happen before its first drawn
        // debug_assert!(other != self.local_player);
        // frame 0 doesnt exist in arra
        for i in self.curr_frame + 1..=frame {
            self.inputs.push_back(PlayerInputs::new(i));
        }
        let last_idx = self.inputs.len() - 1;
        self.inputs[last_idx].insert_player_input(inp, if self.local_player == PlayerID::Player1 {
            PlayerID::Player2
        } else {
            PlayerID::Player1
        });
        self.curr_frame = frame;
    }
    pub fn pop_next_verified_frame(&mut self) -> Option<PlayerInputs> {
        if let Some(front) = self.inputs.front() {
            if front.is_verified(self.player_count) {
                let res = self.inputs.pop_front().unwrap();
                self.last_verified_inputs = res.inputs.clone();
                return Some(res);
            }
        }
        None
    }

    pub fn excluding_iter_after_last_verified(
        &self
    ) -> impl Iterator<Item = (usize, PlayerInputs)> + '_ {
        (0..self.inputs.len()).filter_map(move |index| {
            let frame_input = &self.inputs[index];
            let mut new_input = frame_input.clone();
           // if new_input.is_verified(self.player_count) {
           //     // TODO VERIFY WE DONT NEED THIS
           //     // if our input is None it means the other sim is ahead of us and we can skip this for now ||
           //     //new_input.inputs[self.local_player as usize].is_none()
           //     println!("frame num of verified{}", new_input.frame);
           //     return None;
           // }
            for (player_id, input) in new_input.inputs.iter_mut().enumerate() {
                *input = self.last_verified_inputs[player_id].clone();
            }
            Some((index, new_input))
        })
    }
}
