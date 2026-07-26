# Hello plugin

Demo plugin — registers one `hello` tool with the AI sphere.

## Usage

Call from the agent:

```json
{"action":"hello","params":{"name":"world"}}
```

Returns:

```json
{"who":"world","reply":"Hello, world!"}
```