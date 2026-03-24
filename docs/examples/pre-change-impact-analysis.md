# Example: Pre-change impact analysis

Before making a significant change, use `spelunk` to understand the blast radius.

## Scenario

You need to change the signature of a core function — say, adding a required parameter to `validate_token()`.

## Step 1: Find everything that calls it

```bash
spelunk graph validate_token --kind calls
```

```
Incoming to 'validate_token':
  calls  auth/middleware.go  (handler:45)
  calls  api/routes.go       (apply_auth:112)
  calls  grpc/interceptor.go (unary_auth:67)
```

## Step 2: Understand each call site

```bash
spelunk search "validate_token call site usage" --graph --limit 20
```

## Step 3: Ask about downstream effects

```bash
spelunk ask "If I add a required 'scope' parameter to validate_token, what would I need to update across the codebase?" --context-chunks 30
```

## Step 4: Find the tests

```bash
spelunk search "validate_token test mock"
spelunk ask "How is validate_token tested? Are there mocks or stubs I need to update?"
```

## Step 5: Check for related documentation

```bash
spelunk search "validate_token authentication documentation comment"
```

## Step 6: Create a plan

```bash
spelunk plan create "add scope parameter to validate_token"
# writes docs/plans/add-scope-parameter-to-validate-token.md
```

The generated checklist will include steps like:
- `- [ ] Update validate_token signature in src/auth/token.rs`
- `- [ ] Update call sites in middleware, routes, and interceptor`
- `- [ ] Update test fixtures and mocks`
- etc.

## Step 7: After the change, verify

```bash
spelunk index .
spelunk verify src/auth/token.rs
```

`spelunk verify` re-embeds the changed file and shows its nearest neighbours. If the function is still semantically close to its call sites, it's likely still well-connected in the index.
