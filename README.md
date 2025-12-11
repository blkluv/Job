# jobmcp 💼

## Get started
Clone, Build, Run :

To test:

```bash
npx @modelcontextprotocol/inspector
```

run binary exectuable first, then use your LLM with settings to suit.

I used "Goose" with Gemini, you can choose any LLM that supports MCP

```bash
❯ goose
starting session | provider: google model: gemini-2.5-pro
    session id: 20251208_6
    working directory: /home/oem

goose is running! Enter your instructions, or try asking what goose can do.

Context: ○○○○○○○○○○ 0% (0/1000000 tokens)
( O)> what extensions do you have (mcp)

─── search_available_extensions | extensionmanager ──────────────────────────
query: mcp

I have the following extensions enabled:

*   **jobmcp**: Nostr Jobs MCP Server - Access decentralized job listings from the Nostr network.
*   **extensionmanager**: Extension Management
*   **todo**: Task Management
*   **chatrecall**: Chat Recall

⏱️  Elapsed time: 9.71s
Context: ○○○○○○○○○○ 0% (3052/1000000 tokens)
( O)> use your Nostr Jobs MCP server to find me any jobs related to "Python" and tell me the salaries

─── search_jobs | jobmcp ──────────────────────────
skill: Python

I found two job listings related to Python. The initial search didn't include salary information, so I'll need to look up the details for each one.
─── get_job_details | jobmcp ──────────────────────────
job_id: d80f737bcbd7e57041d0516925b0c737394e6f783d2972c8dc8b2f22ef99c7c9


─── get_job_details | jobmcp ──────────────────────────
job_id: 0491c91d279a9b41258316eb39b5cd1f3348f11343a0a96801d68066543d7208

Here are the details for the two Python-related jobs, including the salary information for the one that provided it:

### 1. Senior Software Engineer at Test Company Ltd
*   **Location**: Remote
*   **Type**: Full-time
*   **Skills**: Rust, C, C++, Python
*   **Salary**: $120,000 - $134,999 USD per year
*   **Description**: 5+ years of professional software engineering experience with a strong track record of backend development using Rust.
*   **Job ID**: `d80f737bcbd7e57041d0516925b0c737394e6f783d2972c8dc8b2f22ef99c7c9`

### 2. Test 5 - Python Developer at Test Company
*   **Location**: Remote
*   **Type**: Full-time
*   **Skills**: C++, Python
*   **Salary**: Not specified in the listing.
*   **Description**: Test Description
*   **Job ID**: `0491c91d279a9b41258316eb39b5cd1f3348f11343a0a96801d68066543d7208`

⏱️  Elapsed time: 23.25s
Context: ○○○○○○○○○○ 0% (4777/1000000 tokens)
( O)> what other skills are most commonly requested along with a Python job?

─── get_stats | jobmcp ──────────────────────────

Based on the current job listings on the Nostr network, the most common skills requested alongside **Python** are:

*   **C++**: This skill appeared in both of the Python job listings I found.
*   **Rust** and **C**: These skills were also mentioned in one of the Python job listings.

⏱️  Elapsed time: 13.33s
Context: ○○○○○○○○○○ 1% (5150/1000000 tokens)

```

# Goose config
```~/.config/goose/config.yaml```

```yaml
extensions:
  jobmcp:
    enabled: true
    type: streamable_http
    name: jobmcp
    description: jobmcp mcp server for searching latest jobs and tool for fetching job details
    uri: http://localhost:9993/mcp
    envs: {}
    env_keys: []
    headers: {}
    timeout: 300
    bundled: null
    available_tools: []
OPENAI_HOST: https://api.openai.com
GOOSE_MODEL: gemini-2.5-pro
GOOSE_PROVIDER: google
OPENAI_BASE_PATH: v1/chat/completions
```

## Demo
https://github.com/user-attachments/assets/65077834-c0bb-4e07-af07-fad76eaeb0b5



