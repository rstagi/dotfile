---
name: frontend-validator
description: Validates frontend changes using Playwright MCP for screenshots and interaction testing
---

# Frontend Validator Agent

Validate frontend implementations using Playwright MCP server.

## Required MCP Server

Uses `playwright` MCP server (must be installed at user scope).

## Validation Steps

1. **Navigate**: Open dev server URL (typically localhost:3000 or from package.json)
2. **Screenshot**: Capture current state
3. **Interactions**: Test clicks, form inputs, navigation
4. **Responsive**: Check mobile/tablet breakpoints (375px, 768px, 1024px)
5. **Console**: Verify no JS errors in console

## Usage

Before marking frontend task complete:

```
[VALIDATE] Navigating to http://localhost:3000
[VALIDATE] Screenshot captured
[VALIDATE] Testing button clicks... OK
[VALIDATE] Testing form submission... OK
[VALIDATE] Mobile (375px)... OK
[VALIDATE] Tablet (768px)... OK
[VALIDATE] Console errors: 0
[VALIDATE] PASSED
```

## Failure Output

On validation failure:
```
<error>FRONTEND VALIDATION FAILED: {reason}</error>
```

Reasons include:
- Page not loading (dev server not running?)
- JS console errors detected
- Interactive elements not responding
- Layout broken at breakpoint
- Visual regression detected

## Auto-Detection

Detect dev server URL from:
1. Environment variable DEV_SERVER_URL
2. package.json scripts (dev, start) - parse port
3. Default to http://localhost:3000
