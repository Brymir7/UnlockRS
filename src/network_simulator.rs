use std::{ cmp::Ordering, collections::BinaryHeap, net::SocketAddr, time::{ Duration, Instant } };

use rand::{ rngs::StdRng, Rng, SeedableRng };

#[derive(Clone)]
struct DelayedMessage {
    data: Vec<u8>,
    addr: SocketAddr, // either src or dst
    delivery_time: Instant,
}

// Custom ordering for min-heap (earlier delivery times come first)

impl Ord for DelayedMessage {
    fn cmp(&self, other: &Self) -> Ordering {
        other.delivery_time.cmp(&self.delivery_time)
    }
}

impl PartialOrd for DelayedMessage {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl PartialEq for DelayedMessage {
    fn eq(&self, other: &Self) -> bool {
        self.delivery_time == other.delivery_time
    }
}

impl Eq for DelayedMessage {}

pub struct NetworkSimulator {
    receive_queue: BinaryHeap<DelayedMessage>,
    send_queue: BinaryHeap<DelayedMessage>,
    rng: rand::rngs::StdRng,
    baseline_latency: u64,
    jitter: u64,
    packet_loss: f32,
}

impl NetworkSimulator {
    pub fn new(seed: u64, baseline_latency: u64, jitter: u64, packet_loss: f32) -> Self {
        Self {
            receive_queue: BinaryHeap::new(),
            send_queue: BinaryHeap::new(),
            rng: StdRng::seed_from_u64(seed),
            baseline_latency,
            jitter,
            packet_loss,
        }
    }

    pub fn modify_baseline_latency(&mut self, delta: i64) {
        self.baseline_latency = ((self.baseline_latency as i64) + delta).max(0) as u64;
        println!("New latency {}", self.baseline_latency);
    }

    pub fn modify_packet_loss(&mut self, delta: f32) {
        self.packet_loss = (self.packet_loss + delta).clamp(0.0, 1.0);
        println!("New packet loss {}", self.packet_loss);
    }

    pub fn modify_jitter(&mut self, delta: i64) {
        self.jitter = ((self.jitter as i64) + delta).max(0) as u64;
        println!("New jitter{}", self.jitter);
    }

    pub fn enqueue_rcv_message(&mut self, data: Vec<u8>, src: SocketAddr) {
        if self.rng.gen::<f32>() >= self.packet_loss {
            let jitter = self.rng.gen_range(0..=self.jitter);
            let delay = self.baseline_latency + jitter;
            let delivery_time = Instant::now() + Duration::from_millis(delay);

            self.receive_queue.push(DelayedMessage {
                data,
                addr: src,
                delivery_time,
            });
        }
    }

    pub fn enqueue_send_message(&mut self, data: Vec<u8>, dst: SocketAddr) {
        if self.rng.gen::<f32>() >= self.packet_loss {
            let jitter = self.rng.gen_range(0..=self.jitter);
            let delay = self.baseline_latency + jitter;
            let delivery_time = Instant::now() + Duration::from_millis(delay);

            self.send_queue.push(DelayedMessage {
                data,
                addr: dst,
                delivery_time,
            });
        }
    }

    pub fn get_ready_receive_messages(&mut self) -> Vec<(Vec<u8>, SocketAddr)> {
        NetworkSimulator::get_ready_messages(&mut self.receive_queue)
    }

    pub fn get_ready_send_messages(&mut self) -> Vec<(Vec<u8>, SocketAddr)> {
        NetworkSimulator::get_ready_messages(&mut self.send_queue)
    }

    fn get_ready_messages(queue: &mut BinaryHeap<DelayedMessage>) -> Vec<(Vec<u8>, SocketAddr)> {
        let now = Instant::now();
        let mut ready_messages = Vec::new();

        while let Some(message) = queue.peek() {
            if message.delivery_time <= now {
                if let Some(msg) = queue.pop() {
                    ready_messages.push((msg.data, msg.addr));
                }
            } else {
                break;
            }
        }

        ready_messages
    }
}
