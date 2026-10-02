# jevify guide

jevify finds the thing you can describe but cannot name, among things that exist. `fill` puts
that thing into a command and runs it; `why`, `pick`, `filter` and `is` read a command's
output and return a line, a subset, a tag per line (`filter --label`) or an exit code. Your
words and
the text need no word in common. When nothing fits, jevify says so and prints nothing. A
probability is the backend's score for that task; it is calibrated only where a measurement
says so.

| Page | Contents |
|:---|:---|
| [Getting started](getting-started.md) | Install, keyless access, first commands, tool selection |
| [Verbs](verbs.md) | `fill`, its markers, `pick --from`, output verbs and utility commands |
| [Kinds](kinds.md) | Every marker kind, a recipe in one line, where `kinds.jsonl` lives, the status line |
| [Agents](agents.md) | One machine envelope, exit codes, capabilities and permissions |
| [How it works](how-it-works.md) | Selection, NONE, thresholds, batching, cache and measurements |
| [Configuration](configuration.md) | Environment variables, global flags and storage |
| [FAQ](faq.md) | Cost, privacy, abstention and model limits |

- [Robot mode](../ROBOT_MODE.md): the agent envelope, exit codes and recovery contract.
- [Privacy](../../PRIVACY.md): evidence sent per verb and raw saved inputs.
- [Changelog](../../CHANGELOG.md): release history.
- [Benchmarks](../../benchmarks/README.md): measurement inputs and conditions.
- [Root-cause cases](../../evals/why/README.md): failing logs and labelled cause ranges.
