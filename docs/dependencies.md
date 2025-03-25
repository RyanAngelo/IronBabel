# Dependencies Documentation

This document provides detailed information about the dependencies used in Iron Babel, including their purposes, version requirements, and important notes about their usage.

## Core Runtime

### tokio (v1.36)
- **Purpose**: Async runtime for handling concurrent operations
- **Features**: Full feature set enabled for maximum functionality
- **Key Usage**:
  - Async I/O operations
  - Task scheduling
  - Network operations
- **Notes**: Using the full feature set to ensure all necessary async capabilities are available
- **License**: MIT
- **Repository**: [tokio-rs/tokio](https://github.com/tokio-rs/tokio)

### async-trait (v0.1)
- **Purpose**: Enables async trait methods in Rust
- **Key Usage**:
  - Protocol trait implementations
  - Gateway trait implementations
- **Notes**: Essential for our trait-based architecture
- **License**: MIT/Apache-2.0
- **Repository**: [dtolnay/async-trait](https://github.com/dtolnay/async-trait)

### futures (v0.3)
- **Purpose**: Utilities for working with futures and streams
- **Key Usage**:
  - Stream processing
  - Future combinators
  - Async utilities
- **License**: MIT/Apache-2.0
- **Repository**: [rust-lang/futures-rs](https://github.com/rust-lang/futures-rs)

## Protocol Support

### axum (v0.7)
- **Purpose**: HTTP server framework
- **Key Usage**:
  - REST API endpoints
  - HTTP server implementation
- **Notes**: Chosen for its ergonomic API and good performance
- **License**: MIT
- **Repository**: [tokio-rs/axum](https://github.com/tokio-rs/axum)

### hyper (v1.1)
- **Purpose**: Low-level HTTP client/server implementation
- **Features**: Full feature set enabled
- **Key Usage**:
  - HTTP client implementation
  - Low-level HTTP operations
- **Notes**: Used by axum and for custom HTTP implementations
- **License**: MIT
- **Repository**: [hyperium/hyper](https://github.com/hyperium/hyper)

### tonic (v0.11)
- **Purpose**: gRPC implementation
- **Key Usage**:
  - gRPC service definitions
  - Protocol buffer handling
- **Notes**: Provides high-performance gRPC support
- **License**: MIT
- **Repository**: [hyperium/tonic](https://github.com/hyperium/tonic)

### async-graphql (v4.0)
- **Purpose**: GraphQL implementation
- **Key Usage**:
  - GraphQL schema definition
  - Query/mutation handling
- **Notes**: Chosen for its async-first design
- **License**: MIT
- **Repository**: [async-graphql/async-graphql](https://github.com/async-graphql/async-graphql)

### rumqtt (v0.31)
- **Purpose**: MQTT protocol implementation
- **Key Usage**:
  - MQTT client/server
  - Pub/sub operations
- **Notes**: Latest stable version with improved async support
- **License**: MIT
- **Repository**: [bytebeamio/rumqtt](https://github.com/bytebeamio/rumqtt)

### ws (v0.9)
- **Purpose**: WebSocket implementation
- **Key Usage**:
  - WebSocket connections
  - Real-time communication
- **Notes**: Used for bidirectional communication
- **License**: MIT
- **Repository**: [snapview/tokio-tungstenite](https://github.com/snapview/tokio-tungstenite)

## Data Handling

### serde (v1.0)
- **Purpose**: Serialization framework
- **Features**: derive feature enabled
- **Key Usage**:
  - Data structure serialization
  - Configuration handling
- **Notes**: Core serialization framework for the project
- **License**: MIT/Apache-2.0
- **Repository**: [serde-rs/serde](https://github.com/serde-rs/serde)

### serde_json (v1.0)
- **Purpose**: JSON serialization support
- **Key Usage**:
  - JSON data handling
  - API responses
- **Notes**: Used for JSON-based protocols
- **License**: MIT/Apache-2.0
- **Repository**: [serde-rs/json](https://github.com/serde-rs/json)

### prost (v0.12)
- **Purpose**: Protocol Buffers support
- **Key Usage**:
  - gRPC message handling
  - Protocol buffer serialization
- **Notes**: Used in conjunction with tonic
- **License**: Apache-2.0
- **Repository**: [tokio-rs/prost](https://github.com/tokio-rs/prost)

## Configuration

### config (v0.13)
- **Purpose**: Configuration management
- **Key Usage**:
  - Application configuration
  - Environment variable handling
- **Notes**: Provides flexible configuration options
- **License**: MIT
- **Repository**: [mehcode/config-rs](https://github.com/mehcode/config-rs)

### serde_yaml (v0.9)
- **Purpose**: YAML configuration support
- **Key Usage**:
  - YAML configuration files
  - Configuration parsing
- **Notes**: Used for human-readable configs
- **License**: MIT/Apache-2.0
- **Repository**: [dtolnay/serde-yaml](https://github.com/dtolnay/serde-yaml)

### toml (v0.8)
- **Purpose**: TOML configuration support
- **Key Usage**:
  - TOML configuration files
  - Configuration parsing
- **Notes**: Used for structured configs
- **License**: MIT/Apache-2.0
- **Repository**: [toml-rs/toml](https://github.com/toml-rs/toml)

## Observability

### tracing (v0.1)
- **Purpose**: Logging and tracing framework
- **Key Usage**:
  - Structured logging
  - Performance tracing
- **Notes**: Core observability framework
- **License**: MIT
- **Repository**: [tokio-rs/tracing](https://github.com/tokio-rs/tracing)

### tracing-subscriber (v0.3)
- **Purpose**: Logging subscriber implementation
- **Key Usage**:
  - Log output configuration
  - Log formatting
- **Notes**: Handles log output and formatting
- **License**: MIT
- **Repository**: [tokio-rs/tracing](https://github.com/tokio-rs/tracing)

### metrics (v0.22)
- **Purpose**: Metrics collection framework
- **Key Usage**:
  - Performance metrics
  - Operation counters
- **Notes**: Core metrics framework
- **License**: MIT
- **Repository**: [metrics-rs/metrics](https://github.com/metrics-rs/metrics)

### metrics-exporter-prometheus (v0.13)
- **Purpose**: Prometheus metrics export
- **Key Usage**:
  - Metrics exposition
  - Monitoring integration
- **Notes**: Enables Prometheus monitoring
- **License**: MIT
- **Repository**: [metrics-rs/metrics](https://github.com/metrics-rs/metrics)

## Error Handling

### thiserror (v1.0)
- **Purpose**: Error type generation
- **Key Usage**:
  - Custom error types
  - Error handling
- **Notes**: Used for type-safe error handling
- **License**: MIT/Apache-2.0
- **Repository**: [dtolnay/thiserror](https://github.com/dtolnay/thiserror)

### anyhow (v1.0)
- **Purpose**: Error handling utilities
- **Key Usage**:
  - Error propagation
  - Error context
- **Notes**: Used for flexible error handling
- **License**: MIT/Apache-2.0
- **Repository**: [dtolnay/anyhow](https://github.com/dtolnay/anyhow)

## Middleware

### tower (v0.4)
- **Purpose**: Middleware framework
- **Key Usage**:
  - Request/response middleware
  - Service composition
- **Notes**: Core middleware framework
- **License**: MIT
- **Repository**: [tower-rs/tower](https://github.com/tower-rs/tower)

### tower-http (v0.5)
- **Purpose**: HTTP-specific middleware
- **Features**: trace, cors
- **Key Usage**:
  - HTTP middleware
  - CORS handling
  - Request tracing
- **Notes**: Provides HTTP-specific middleware capabilities
- **License**: MIT
- **Repository**: [tower-rs/tower-http](https://github.com/tower-rs/tower-http)

## Testing

### tokio-test (v0.4)
- **Purpose**: Async testing utilities
- **Key Usage**:
  - Async test helpers
  - Mock runtime
- **Notes**: Essential for async testing
- **License**: MIT
- **Repository**: [tokio-rs/tokio](https://github.com/tokio-rs/tokio)

### wiremock (v0.5)
- **Purpose**: HTTP mocking
- **Key Usage**:
  - HTTP request mocking
  - Integration testing
- **Notes**: Used for HTTP service mocking
- **License**: MIT
- **Repository**: [LukeMathWalker/wiremock-rs](https://github.com/LukeMathWalker/wiremock-rs)

## License Summary
All dependencies are either MIT or Apache-2.0 licensed, ensuring compatibility with our MIT project license. The majority of dependencies are MIT licensed, with some using dual MIT/Apache-2.0 licensing.

## Version Management

### Version Pinning
- All dependencies use specific versions to ensure reproducible builds
- Versions are chosen based on stability and feature requirements
- Regular updates are planned to keep dependencies current

### Security Considerations
- Regular security audits of dependencies
- Immediate updates for security-critical dependencies
- Version constraints are carefully managed

### Performance Impact
- Dependencies are chosen with performance in mind
- Heavy dependencies are avoided where possible
- Async-first dependencies are preferred

## Updating Dependencies

### Process
1. Review changelog for breaking changes
2. Update in development environment
3. Run full test suite
4. Update documentation
5. Deploy to staging
6. Monitor for issues
7. Deploy to production

### Guidelines
- Keep dependencies up to date
- Test thoroughly after updates
- Document any breaking changes
- Consider performance impact
- Monitor security advisories 