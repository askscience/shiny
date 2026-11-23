# Calculator plugin — agent tools

You can do arithmetic and scientific math reliably with the `calculator_eval` tool — it evaluates an expression with a small, safe math engine and records the result in the user's calculator history.

**The JSON contract:**
- `calculator_eval` — evaluate one expression: `{"action":"calculator_eval","params":{"expression":"(2+3)*4"}}` → returns `{ expression, result, result_text }` (e.g. `result: 20`, `result_text: "20"`). The result is also saved to the calculator history shown in the Calculator window.
- `calculator_history` — list recent calculations: `{"action":"calculator_history","params":{"limit":10}}` (`limit` optional, default 20, 1–100) → returns `{ history: [{expression,result,at}, …], count }`.
- `calculator_clear_history` — clear the user's calculation history: `{"action":"calculator_clear_history","params":{}}` → returns `{ cleared: true }`.

**Supported syntax:**
- Operators: `+ - * / % ^ !` and parentheses. `^` is power, `!` is factorial (non-negative integer), `-x` negates.
- `%` closing a term is a percentage, as on a pocket calculator: `50+10%` is 55, `50-10%` is 45, `200*15%` is 30, `3%` is 0.03. Between two numbers (`10%3`) it is the remainder; `mod(a,b)` says that unambiguously.
- A missing `*` is implied next to a constant, a function or a bracket: `2pi`, `3(4+1)`, `2sqrt(9)`.
- Scientific functions (one argument): `sin cos tan` (radians), `sind cosd tand` (degrees), `asin acos atan`, `asind acosd atand` (result in degrees), `sinh cosh tanh asinh acosh atanh`, `sqrt cbrt`, `ln` (natural log), `log` (base-10) / `log2`, `exp`, `abs`, `floor`, `ceil`, `round`, `trunc`, `sign`, `deg` (radians→degrees), `rad` (degrees→radians), `fact` / `factorial`.
- Two-argument functions: `atan2(y,x)`, `pow(a,b)`, `mod(a,b)`, `hypot(a,b)`, and `log(base,x)`.
- Constants: `pi`, `e`, `tau`, `phi`.

Rules:
- Always pass a single complete expression in `expression` — no need to break a computation into steps unless the user asks for it.
- When the user asks a math question ("what is …", "convert …", "how much is …"), prefer `calculator_eval` over doing the arithmetic yourself, so the answer matches the calculator exactly.
- Report the `result_text` in your reply. If the expression fails, read the error message and retry with a corrected expression.
