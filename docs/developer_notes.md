# Iron Babel Developer Notes

## Architecture Overview

### Gateway Concept
The gateway is a middleware service that acts as a translation layer between different communication protocols. It's designed to be a universal translator for different communication protocols, allowing services to communicate without needing to understand each other's protocols.

#### Key Responsibilities
- Protocol translation (e.g., HTTP ↔ gRPC, GraphQL ↔ REST)
- Request/response transformation
- Schema management
- Error handling
- Metrics collection
- Logging
- Configuration management

#### Real-World Example
Consider a scenario with:
- Frontend service using GraphQL
- Backend service using gRPC

The gateway would:
1. Receive a GraphQL query from the frontend
2. Translate it into a gRPC request
3. Forward it to the backend
4. Receive the gRPC response
5. Translate it back to GraphQL format
6. Send it back to the frontend

### Core Components

#### Gateway Trait
```rust
pub trait Gateway: Send + Sync {
    async fn start(&self) -> Result<()>;
    async fn stop(&self) -> Result<()>;
    fn protocols(&self) -> Vec<Arc<dyn Protocol>>;
}
```
This defines the basic lifecycle and protocol management of the gateway.

#### Protocol Gateway Trait
```rust
pub trait ProtocolGateway: Send + Sync {
    async fn handle_request(&self, request: Vec<u8>) -> Result<Vec<u8>>;
    fn protocol(&self) -> Arc<dyn Protocol>;
}
```
These are specialized implementations for each protocol (HTTP, gRPC, GraphQL, etc.).

## Project Structure
```
iron-babel/
├── src/
│   ├── config/        # Configuration management
│   ├── core/          # Core gateway functionality
│   ├── gateway/       # Protocol-specific gateway implementations
│   ├── protocols/     # Individual protocol implementations
│   ├── schema/        # Schema management
│   ├── transform/     # Data transformation
│   └── utils/         # Common utilities
├── docs/             # Documentation
└── tests/            # Integration tests
```

## Dependencies

### Core Runtime
- `tokio` (v1.36) - Async runtime with full features enabled
- `async-trait` (v0.1) - Async trait support
- `futures` (v0.3) - Future utilities

### Protocol Support
- `axum` (v0.7) - HTTP server framework
- `hyper` (v1.1) - HTTP client/server
- `tonic` (v0.11) - gRPC implementation
- `async-graphql` (v4.0) - GraphQL implementation
- `rumqtt` (v0.31) - MQTT implementation
- `ws` (v0.9) - WebSocket implementation

### Data Handling
- `serde` (v1.0) - Serialization framework
- `serde_json` (v1.0) - JSON serialization
- `prost` (v0.12) - Protocol Buffers support

### Configuration
- `config` (v0.13) - Configuration management
- `serde_yaml` (v0.9) - YAML support
- `toml` (v0.8) - TOML support

### Observability
- `tracing` (v0.1) - Logging framework
- `tracing-subscriber` (v0.3) - Logging subscriber
- `metrics` (v0.22) - Metrics collection
- `metrics-exporter-prometheus` (v0.13) - Prometheus metrics export

### Error Handling
- `thiserror` (v1.0) - Error type generation
- `anyhow` (v1.0) - Error handling utilities

### Middleware
- `tower` (v0.4) - Middleware framework
- `tower-http` (v0.5) - HTTP middleware

### Testing
- `tokio-test` (v0.4) - Async testing utilities
- `wiremock` (v0.5) - HTTP mocking

## Design Decisions
1. Using async/await for all I/O operations
2. Protocol-agnostic request/response handling using `Vec<u8>`
3. Trait-based design for extensibility
4. Thread-safe components using `Send + Sync`
5. Centralized error handling and logging
6. Comprehensive metrics and tracing support
7. Flexible configuration management
8. Middleware-based architecture for extensibility

## Future Considerations
- Add support for more protocols
- Implement caching layer
- Add circuit breaking
- Support for custom protocol implementations
- Performance optimization for high-throughput scenarios
- Add WebSocket support for real-time communication
- Implement rate limiting and throttling
- Add authentication/authorization middleware

## Testing Strategy

### Test Structure
The project uses a comprehensive testing approach with three main types of tests:

1. **Unit Tests**
   - Located alongside the source code in each module
   - Test individual components in isolation
   - Use mock objects for dependencies

2. **Integration Tests**
   - Located in the `tests/` directory
   - Test multiple components working together
   - Verify end-to-end functionality

3. **Test Utilities**
   - Located in `src/test_utils/`
   - Provide common testing infrastructure
   - Include mock implementations of core traits

### Mock Objects
The project includes several mock implementations for testing:
- Mock protocols
- Mock gateways
- Mock configuration
- Mock metrics collectors

## Development Guidelines

### Code Style
- Follow Rust standard formatting (rustfmt)
- Use clippy for linting
- Document all public APIs
- Write unit tests for new functionality

### Performance Considerations
- Use async/await for I/O operations
- Implement proper error handling
- Monitor memory usage
- Profile critical paths

### Security
- Validate all inputs
- Sanitize outputs
- Use secure defaults
- Implement proper authentication/authorization

### Monitoring
- Use structured logging
- Collect metrics for all operations
- Monitor resource usage
- Set up alerts for critical issues 