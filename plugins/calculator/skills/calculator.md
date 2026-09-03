# Calculator plugin — agent tools

You can do arithmetic and scientific math reliably with the `calculator_eval` tool — it evaluates an expression with a small, safe math engine and records the result in the user's calculator history.

**The JSON contract:**
- `calculator_eval` — evaluate one expression: `{"action":"calculator_eval","params":{"expression":"(2+3)*4"}}` → returns `{ expression, result, result_text }` (e.g. `result: 20`, `result_text: "20"`). The result is also saved to the calculator history shown in the Calculator window.
- `calculator_history` — list recent calculations: `{"action":"calculator_history","params":{"limit":10}}` → returns `{ history: [{expression,result,at}, …], count }`.
- `calculator_clear_history` — clear the user's calculation history: `{"action":"calculator_clear_history","params":{}}`.

**Supported syntax:**
- Operators: `+ - * / % ^ !` and parentheses. `^` is power, `!` is factorial (non-negative integer), `%` is remainder. `-x` negates.
- Scientific functions (one argument): `sin cos tan` (radians), `sind cosd tand` (degrees), `asin acos atan`, `sinh cosh tanh asinh acosh atanh`, `sqrt cbrt`, `ln` (natural log), `log` (base-10) / `log2`, `exp`, `abs`, `floor`, `ceil`, `round`, `trunc`, `sign`, `deg` (radians→degrees), `rad` (degrees→radians), `fact` (factorial).
- Two-argument functions: `atan2(y,x)`, `pow(a,b)`, `mod(a,b)`, `hypot(a,b)`, and `log(base,x)`.
- Constants: `pi`, `e`, `tau`, `phi`.

Rules:
- Always pass a single complete expression in `expression` — no need to break a computation into steps unless the user asks for it.
- When the user asks a math question ("what is …", "convert …", "how much is …"), prefer `calculator_eval` over doing the arithmetic yourself, so the answer matches the calculator exactly.
- Report the `result_text` in your reply. If the expression fails, read the error message and retry with a corrected expression.
