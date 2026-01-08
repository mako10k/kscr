# Language BNF

This document defines the complete grammar and lexical structure of the language.

## Purpose
- Provides the formal BNF specification for syntax, types, expressions, patterns, and modules.
- For language semantics and evaluation, see `LanguageSemantics.md`.
- For type system details, see `TypeSystem.md`.
- For internal representation, see `IntermediateRepresentation.md`.

---

# BNF Specification for Lazy Evaluation Scripting Language

## Lexical Elements

<integer> ::= [0-9]+
<float> ::= [0-9]+ '.' [0-9]+ ([eE] [+-]? [0-9]+)?
<char> ::= '\'' <character or escape sequence> '\''
<bool> ::= 'True' | 'False'
<unit> ::= '()'
<identifier> ::= [a-zA-Z_][a-zA-Z0-9_]*
<string> ::= '"' {<character or escape sequence>} '"'

---

## Comments and Shebang

- **Line Comment**: Begins with `--` and continues to the end of the line.
- **Block Comment**: Enclosed by `{-` and `-}`; can be nested.
- Comments are ignored by the parser and do not affect program semantics.

- **Shebang Line**: If the first line of a source file begins with `#`, it is treated as a special comment (shebang) and ignored by the parser. This allows scripts to be run directly as executables on Unix-like systems (e.g., `#!/usr/bin/env mylang`).

### Example
```
#!/usr/bin/env mylang
-- This is a line comment
{- This is a
	block comment -}
main = print "Hello"
```

---

## Types

type ::= unit_type
	| integer_type
	| char_type
	| bool_type
	| float64_type
	| string_type
	| tuple_type
	| list_type
	| record_type
	| function_type
	| data_type
	| type_variable
	| type_hole

unit_type ::= '()'
integer_type ::= 'Integer'
char_type ::= 'Char'
bool_type ::= 'Bool'
float64_type ::= 'Float64'
string_type ::= 'String'   # alias for [Char]
tuple_type ::= '(' type {',' type} ')'
list_type ::= '[' type ']'
record_type ::= '{' field_type_list '}'
field_type_list ::= field_type {',' field_type}
field_type ::= <identifier> ':' type
function_type ::= type '->' type
data_type ::= <identifier> {type}
type_variable ::= <identifier>
type_hole ::= '?' | '?' <identifier>

---

## Expressions

expr ::= literal
	| variable
	| tuple_expr
	| list_expr
	| record_expr
	| lambda_expr
	| application
	| infix_application
	| data_ctor_expr
	| type_annotation
	| let_expr
	| where_expr
	| if_expr
	| case_expr
	| list_comprehension
	| do_block

literal ::= <integer> | <float> | <char> | <bool> | <unit> | <string>
variable ::= <identifier>
tuple_expr ::= '(' expr {',' expr} ')'
list_expr ::= '[' expr_list ']'
expr_list ::= expr {',' expr}
record_expr ::= '{' field_expr_list '}'
field_expr_list ::= field_expr {',' field_expr}
field_expr ::= <identifier> ':' expr
lambda_expr ::= '\' pattern_list '->' expr
pattern_list ::= pattern {pattern}
application ::= expr expr {expr}
infix_application ::= expr infix_op expr
infix_op ::= '`' <identifier> '`' | '+' | '-' | '*' | '/' | '==' | '/=' | '<' | '>' | '<=' | '>=' | '&&' | '||' | ':' | '++' | ...
data_ctor_expr ::= <identifier> {expr}
type_annotation ::= expr '::' type
let_expr ::= 'let' binding_list 'in' expr
where_expr ::= expr 'where' binding_list
binding_list ::= binding {';' binding}
binding ::= pattern '=' expr
if_expr ::= 'if' expr 'then' expr 'else' expr
case_expr ::= 'case' expr 'of' case_alt_list
case_alt_list ::= case_alt {';' case_alt}
case_alt ::= pattern [guard] '->' expr
guard ::= '|' expr
list_comprehension ::= '[' expr '|' generator_list ']'
generator_list ::= generator {',' generator}
generator ::= pattern '<-' expr | expr
do_block ::= 'do' indent_block
indent_block ::= INDENT {statement} DEDENT
statement ::= pattern '<-' expr | expr

---

## Patterns

pattern ::= literal
	   | variable_pattern
	   | wildcard_pattern
	   | as_pattern
	   | hole_pattern
	   | tuple_pattern
	   | list_pattern
	   | cons_pattern
	   | record_pattern_strict
	   | record_pattern_loose
	   | data_pattern
	   | or_pattern
	   | view_pattern

variable_pattern ::= <identifier>
wildcard_pattern ::= '_'
as_pattern ::= <identifier> '@' pattern
hole_pattern ::= '?' | '?' <identifier>
tuple_pattern ::= '(' pattern {',' pattern} ')'
list_pattern ::= '[' pattern_list ']'
pattern_list ::= pattern {',' pattern}
cons_pattern ::= pattern ':' pattern
record_pattern_strict ::= '{' field_pattern_list '}'
field_pattern_list ::= field_pattern {',' field_pattern}
field_pattern ::= <identifier> ':' pattern
record_pattern_loose ::= '{' field_pattern_list ',' '...' '}'
data_pattern ::= <identifier> {pattern}
or_pattern ::= pattern '|' pattern
view_pattern ::= pattern '<-' expr

---

## Data Type Declaration

data_decl ::= 'data' type_name type_vars '=' ctor_list
type_name ::= <identifier>
type_vars ::= <identifier> {<identifier>}
ctor_list ::= ctor {'|' ctor}
ctor ::= <identifier> ctor_args
ctor_args ::= /* empty */ | type_atom {type_atom}
type_atom ::= <identifier> | '(' type ')'

# Example:
# data Maybe a = Nothing | Just a
# data Either a b = Left a | Right b

---

## Type Alias Declaration

Type aliases (type synonyms) provide a new name for an existing type.

type_alias_decl ::= 'type' type_name type_vars '=' type

# Example:
# type String = [Char]
# type Pair a b = (a, b)

---

## Module and Indent Grouping

module_decl ::= 'module' <identifier> 'where' module_block
module_block ::= INDENT {module_stmt} DEDENT
module_stmt ::= import_decl | export_decl | data_decl | type_alias_decl | binding

import_decl ::= 'import' <identifier> [ 'as' <identifier> ]
export_decl ::= 'export' export_list
export_list ::= <identifier> {',' <identifier>}

### INDENT/DEDENT Tokens
- **INDENT**: Marks the beginning of an indented block. Generated when indentation level increases.
- **DEDENT**: Marks the end of an indented block. Generated when indentation level decreases.
- Consistent indentation is required within a block; mixing tabs and spaces is an error.
- The lexer tracks indentation levels and generates INDENT/DEDENT tokens accordingly.

---

## Notes

### Operator Precedence and Associativity
Infix operators follow standard precedence and associativity rules:
- Multiplicative operators (`*`, `/`, `mod`) bind tighter than additive (`+`, `-`)
- Comparison operators (`==`, `<`, etc.) have lower precedence
- Logical operators (`&&`, `||`) have the lowest precedence
- Function application has the highest precedence
- Custom infix operators can have their precedence/associativity defined via fixity declarations: `infixl n op`, `infixr n op`, `infix n op` (parser-level)

### Infix Notation
Any binary function can be used as an infix operator by enclosing it in backticks: ``a `f` b`` is equivalent to `f a b`. See `IntermediateRepresentation.md` for details on infix operator handling in the IR.

### Sections (operator prefixification / partial application)
- `(op)` turns an operator into a normal function.
- `(op x)` and `(x op)` are supported as operator sections and desugar to lambdas.

---
