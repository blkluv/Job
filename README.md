# jobmcp

Clone, Build, Run :

<img width="1604" height="882" alt="nos_2" src="https://github.com/user-attachments/assets/9e159a0c-dc8b-4728-83db-4778c5c31578" />

---


https://github.com/user-attachments/assets/dc896463-0208-4e74-8b06-86c324298434


we'll use the 'old' sse transport to begin building

To test:

```bash
npx @modelcontextprotocol/inspector
```

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

