# Example: Onboarding a new codebase

You've been handed a large project you've never seen before. Here's how to get up to speed quickly with `spelunk`.

## Step 1: Index it

```bash
spelunk index /path/to/project
# Indexing 847 files… done in 2m 14s
# 12,341 chunks stored, 12,341 embeddings
```

## Step 2: Get a high-level overview

```bash
spelunk ask "Give me a high-level overview of this codebase. What does it do and how is it structured?"
```

## Step 3: Understand the entry points

```bash
spelunk ask "Where does the application start? What is the main entry point?"
spelunk search "main function application startup"
```

## Step 4: Find the key abstractions

```bash
spelunk ask "What are the main abstractions and domain objects in this codebase?"
spelunk search "core interfaces traits protocols" --graph
```

## Step 5: Understand the data layer

```bash
spelunk ask "How is data persisted? What database or storage layer is used?"
spelunk graph Database --kind calls
```

## Step 6: Find the API surface

```bash
spelunk search "HTTP handler route endpoint"
spelunk ask "What external APIs or endpoints does this service expose?"
```

## Step 7: Understand error handling

```bash
spelunk ask "What is the error handling strategy? How are errors propagated and surfaced to users?"
```

## Step 8: Store what you've learned

```bash
spelunk memory add \
  --title "This service is a payment processor wrapping Stripe" \
  --body "Entry point: cmd/server/main.go. Core domain: pkg/payments/. REST API in pkg/api/. PostgreSQL via GORM." \
  --kind context \
  --tags architecture,overview

spelunk memory add \
  --title "Errors are wrapped with pkg/errors and logged at the handler boundary" \
  --kind context \
  --tags error-handling
```

Now future sessions (and future agents) can start from your notes rather than re-discovering the same things.

## Step 9: Check what tests exist

```bash
spelunk search "test suite integration test unit test"
spelunk ask "What testing strategy is used? Are there unit tests, integration tests, or end-to-end tests?"
```

## Step 10: Understand the build and deploy story

```bash
spelunk ask "How is this project built and deployed? Are there Makefiles, Docker files, or CI configurations?"
```

After this session you'll have a solid mental model and a set of memory entries that make every future session faster.
