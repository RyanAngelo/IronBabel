# Iron Babel

IronBabel is a protocol bridge, not just an API gateway. It connects HTTP APIs to gRPC, GraphQL, WebSockets, MQTT, and ZeroMQ in one lightweight runtime.

## Features

- Protocol translation between:
  - REST/HTTP ↔ GraphQL
  - REST/HTTP ↔ gRPC
  - WebSockets ↔ HTTP/SSE
  - WebSockets ↔ REST polling
  - HTTP ↔ MQTT publish/webhook flows
- Automatic schema discovery and generation
- Request/response transformation
- Developer-friendly configuration
- Comprehensive monitoring and metrics via built-in admin dashboard
- Advanced features like rate limiting and circuit breaking

## Monitoring Dashboard

![IronBabel Monitoring Dashboard](https://www.ryanangelo.com/projects/iron-babel/screenshots/monitoring-dashboard.png)

The built-in admin dashboard is available at `http://<host>:<port>/admin/` and provides real-time request metrics, latency percentiles, error rates, and per-route stats.

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
