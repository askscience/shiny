# Hello plugin

Demo plugin — registers one `hello` tool with the AI assistant.

## Usage

Call from the agent:

```json
{"action":"hello","params":{"name":"world"}}
```

Returns:

```json
{"who":"world","reply":"Hello, world!"}
```