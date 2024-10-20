# UnlockRS
![Example Simulation](example.png)

UnlockRS is a real-time networked multiplayer game using UDP. The project implements a verified vs predicted simulation model with a relay server.

## Key Features

- **Deterministic Simulation**: The simulation is designed to be deterministic, allowing it to run the same way on both clients using only player inputs.
- **Verified vs Predicted Model**: This model enhances gameplay by verifying player actions and predicting their outcomes to reduce latency and improve responsiveness.
- **Relay Server**: The relay server facilitates communication between clients and can be expanded to handle synchronization (using state checksums).
