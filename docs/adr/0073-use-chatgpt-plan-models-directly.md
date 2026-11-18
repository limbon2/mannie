# ADR-0073: Use ChatGPT plan models directly

|          |                                                                                                                        |
| -------- | ---------------------------------------------------------------------------------------------------------------------- |
| Status   | Accepted (D68)                                                                                                         |
| Date     | 2026-11-18                                                                                                             |
| Deciders | Owner                                                                                                                  |
| Related  | [ADR-0062](0062-let-the-owner-pay-for-model-calls-with-a-chatgpt-plan.md), [ChatGPT plan](../features/chatgpt-plan.md) |

## Context

Signing in gives Mannie the account's model catalog. Previously the owner also had to create a profile for each model and enter token limits for models Mannie did not recognize. The owner wants to send the first message immediately after signing in.

OpenAI documents how to read the selected account's catalog, keep the visible entries in order and use the selected model's slug for inference. A listed entry can identify a model without giving its limits or supported reasoning efforts. Verified on 2026-11-18 against [Models and inference](https://developers.openai.com/siwc/token-sharing-open-source/models-and-inference).

## Decision

1. Offer the account's listed models directly in conversations and the agent editor. When there is no chosen model, agent profile or configured default, use the first listed plan model.
2. Give catalog models stable internal profile IDs. Saved chats and agents can keep their selection when a model becomes unavailable. Ask the owner to sign in or select another model before sending in that case. Owner-created profiles keep their own configuration.
3. Supersede the rule for model limits in ADR-0062 decision 11 for automatic plan models. Use valid catalog limits, then the verified limits of a known model. Otherwise budget locally for a 32,768-token context window and reserve 4,096 tokens for output. The request to the plan still omits an output limit.
4. A catalog model uses the same master-only credentials, account binding for encrypted reasoning and restrictions on worker delegation as an owner-created plan profile.

## Consequences

New models can receive messages before Mannie has verified their limits. An estimated local budget can compact a conversation early or exceed a server's actual capacity. An owner can still configure a profile with explicit limits.

## Alternatives considered

| Option                                            | Reason for the decision                                                                                                  |
| ------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------ |
| Ask for limits on first use                       | The owner chose automatic budgets so a newly listed model can answer immediately.                                        |
| Create an editable profile for every listed model | The account controls its catalog. Owner-created profiles can use distinct names, limits and defaults for the same model. |

## Validation

Use an account fixture that lists a model with no limits. Sign in, send the first message and check the model in the recorded plan request. Cover catalog changes, saved selections and owner-created profiles separately.
