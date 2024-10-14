use std::collections::VecDeque;

use crate::{ types::PlayerInput, MAX_PLAYER_COUNT };

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

    fn insert_player_input(&mut self, input: Vec<PlayerInput>, player_id: usize) {
        self.inputs[player_id] = Some(input);
    }

    fn is_verified(&self) -> bool {
        self.inputs.iter().all(|input| input.is_some())
    }
}

#[derive(Debug)]
pub struct InputBuffer {
    inputs: VecDeque<PlayerInputs>,
    last_verified_index: usize,
    last_verified_inputs: [Option<Vec<PlayerInput>>; MAX_PLAYER_COUNT as usize],
}

impl InputBuffer {
    pub fn new() -> Self {
        InputBuffer {
            inputs: VecDeque::new(),
            last_verified_index: 0,
            last_verified_inputs: [None, None],
        }
    }

    pub fn insert_player_input(&mut self, inp: Vec<PlayerInput>, frame: u32, player_id: usize) {
        while self.inputs.len() <= (frame as usize) {
            self.inputs.push_back(PlayerInputs::new(self.inputs.len() as u32));
        }
        self.inputs[frame as usize].insert_player_input(inp, player_id);
        self.update_last_verified_index();
    }

    fn update_last_verified_index(&mut self) {
        while
            self.last_verified_index < self.inputs.len() &&
            self.inputs[self.last_verified_index].is_verified()
        {
            for (player_id, input) in self.inputs[self.last_verified_index].inputs
                .iter()
                .enumerate() {
                if let Some(input) = input {
                    self.last_verified_inputs[player_id] = Some(input.clone());
                }
            }
            self.last_verified_index += 1;
        }
    }

    pub fn get_first_verified_input(&self) -> Option<&PlayerInputs> {
        if self.last_verified_index > 0 {
            self.inputs.get(self.last_verified_index - 1)
        } else {
            None
        }
    }

    fn remove_verified_inputs(&mut self) {
        if self.last_verified_index > 0 {
            self.inputs.drain(0..self.last_verified_index);
            self.last_verified_index = 0;
        }
    }

    pub fn iter_from_last_verified(&self) -> impl Iterator<Item = (usize, PlayerInputs)> + '_ {
        (0..self.inputs.len() - self.last_verified_index).map(move |i| {
            let index = self.last_verified_index + i;
            let frame_input = &self.inputs[index];
            let mut new_input = frame_input.clone();
            for (player_id, input) in new_input.inputs.iter_mut().enumerate() {
                if input.is_none() {
                    *input = self.last_verified_inputs[player_id].clone();
                }
            }
            (index, new_input)
        })
    }
}
