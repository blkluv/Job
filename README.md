# jobmcp


https://github.com/user-attachments/assets/608dec80-083e-4a9b-a292-10a46c4a63c3



sse to begin

npx @modelcontextprotocol/inspector

http://127.0.0.1:8000/sse

Connection type : via proxy

run binary exectuable first, then use your LLM with settings to suit.

I used "Goose" with Gemini, you can choose any LLM that supports MCP

```bash
❯ cat config.yaml
extensions:
  jobmcp:
    enabled: true
    envs: {}
    name: jobmcp
    timeout: 30000
    type: sse
    uri: http://localhost:8000/sse
OPENAI_HOST: https://api.openai.com
GOOSE_MODEL: gemini-2.5-pro
GOOSE_PROVIDER: google
OPENAI_BASE_PATH: v1/chat/completions

```

