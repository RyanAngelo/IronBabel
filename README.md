# Iron Babel

A powerful cross-protocol API gateway written in Rust that enables seamless communication between services using different protocols.

## Features

- Protocol translation between:
  - REST/HTTP ↔ GraphQL
  - REST/HTTP ↔ gRPC
  - WebSockets ↔ HTTP/SSE
  - WebSockets ↔ REST polling
  - MQTT ↔ HTTP webhooks
- Automatic schema discovery and generation
- Request/response transformation
- Developer-friendly configuration
- Comprehensive monitoring and metrics
- Advanced features like rate limiting and circuit breaking

## Project Structure

The project is organized into several key components:

- `config/`: Configuration management
- `core/`: Core gateway functionality
- `gateway/`: Protocol-specific gateway implementations
- `protocols/`: Individual protocol implementations
- `schema/`: Schema management and generation
- `transform/`: Data transformation utilities
- `utils/`: Common utilities and helpers

## Getting Started

1. Ensure you have Rust installed (version 1.75 or later)
2. Clone the repository
3. Build the project:
   ```bash
   cargo build
   ```
4. Run tests:
   ```bash
   cargo test
   ```

## Configuration

Configuration can be provided through:
- YAML/TOML files
- Environment variables
- Command-line arguments

## Contributing

Contributions are welcome! Please feel free to submit a Pull Request.

## License

This project is licensed under the MIT License - see the LICENSE file for details.
