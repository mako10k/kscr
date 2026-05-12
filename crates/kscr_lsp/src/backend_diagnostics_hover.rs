use crate::backend_helpers::{
    contextual_ident_kind_at_offset, create_diagnostic, qualified_ident_at_offset,
    qualified_ident_parts_at_offset, span_to_range,
};
use crate::vfs::Document;
use kscr::{ast, error::Error as KscrError, lexer, parser, types};
use std::time::{SystemTime, UNIX_EPOCH};
use tower_lsp::lsp_types::*;

pub(super) fn compute_diagnostics(doc: &Document) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();

    match lexer::lex(&doc.text) {
        Err(e) => {
            diagnostics.push(create_diagnostic(doc, &e, DiagnosticSeverity::ERROR));
            return diagnostics;
        }
        Ok(_tokens) => match parser::parse_module(&doc.text) {
            Err(e) => {
                diagnostics.push(create_diagnostic(doc, &e, DiagnosticSeverity::ERROR));
                return diagnostics;
            }
            Ok(_module) => {
                if let Err(e) = typecheck_document_text(&doc.uri, &doc.text) {
                    diagnostics.push(create_diagnostic(doc, &e, DiagnosticSeverity::ERROR));
                }
            }
        },
    }

    diagnostics
}

pub(super) fn typecheck_document_text(uri: &Url, text: &str) -> std::result::Result<(), KscrError> {
    typecheck_document_typed(uri, text).map(|_| ())
}

pub(super) fn typecheck_document_typed(
    uri: &Url,
    text: &str,
) -> std::result::Result<types::TypedModule, KscrError> {
    let path = uri
        .to_file_path()
        .map_err(|_| KscrError::msg("Cannot convert URI to file path"))?;

    if path.exists() {
        return types::typecheck_file(&path);
    }

    let parent = path
        .parent()
        .filter(|p| p.exists())
        .map(|p| p.to_path_buf())
        .unwrap_or_else(std::env::temp_dir);

    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("unsaved");
    let pid = std::process::id();
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);

    let tmp_path = parent.join(format!(".kscr-lsp-{stem}-{pid}-{nanos}.ks"));
    std::fs::write(&tmp_path, text)?;

    let res = types::typecheck_file(&tmp_path);
    let _ = std::fs::remove_file(&tmp_path);
    res
}

pub(super) fn hover_in_doc(doc: &Document, pos: Position) -> Option<Hover> {
    let off = doc.position_to_offset(pos.line, pos.character)?;
    if let Some(module_hover) = hover_module_qualifier_in_doc(doc, off) {
        return Some(module_hover);
    }

    let (name, name_span) = qualified_ident_at_offset(&doc.text, off)?;
    let typed = typecheck_document_typed(&doc.uri, &doc.text).ok();

    if let Some(param_hover) = hover_parameter_in_doc(doc, off, &name, typed.as_ref()) {
        return Some(param_hover);
    }
    if let Some(param_hover) = hover_parameter_use_in_doc(doc, off, typed.as_ref()) {
        return Some(param_hover);
    }
    if let Some(param_hover) = hover_method_parameter_use_in_doc(doc, off, typed.as_ref()) {
        return Some(param_hover);
    }
    if let Some(param_hover) = hover_line_parameter_use_in_doc(doc, off, typed.as_ref()) {
        return Some(param_hover);
    }
    if let Some(local_hover) = hover_local_binding_in_doc(doc, off, &name, typed.as_ref()) {
        return Some(local_hover);
    }

    let module = parser::parse_module(&doc.text).ok();
    let method_name = name.rsplit('.').next().unwrap_or(&name);
    let method_ty = typed
        .as_ref()
        .and_then(|tm| tm.class_methods.get(method_name).cloned());

    let kind = contextual_ident_kind_at_offset(&doc.text, off)
        .or_else(|| {
            module
                .as_ref()
                .and_then(|m| crate::backend_goto_completion::super_classify_toplevel_symbol(m, &name))
        })
        .or_else(|| method_ty.as_ref().map(|_| "class method"))
        .unwrap_or("identifier");

    let ty = typed
        .as_ref()
        .and_then(|tm| tm.inferred.get(&name).cloned())
        .or(method_ty)
        .or_else(|| types::builtin_hover_scheme(&name))
        .map(|s| types::format_pretty_scheme(&s));
    let doc_comment = typed.as_ref().and_then(|tm| tm.docs.get(&name).cloned());
    let builtin_kind = types::builtin_hover_kind(&name);

    let range = span_to_range(doc, name_span);
    let hover_kind = builtin_kind.map(|k| k.hover_label()).unwrap_or(kind);
    let mut value = match ty {
        Some(ty) if hover_kind != "identifier" => {
            format!("```kscr\n{hover_kind} {name} :: {ty}\n```")
        }
        Some(ty) => format!("```kscr\n{name} :: {ty}\n```"),
        None => format!(
            "```kscr\n{} {}\n```",
            hover_kind,
            name
        ),
    };
    if let Some(doc_comment) = doc_comment {
        value.push_str("\n---\n");
        value.push_str(&doc_comment);
    }

    Some(Hover {
        contents: HoverContents::Markup(MarkupContent {
            kind: MarkupKind::Markdown,
            value,
        }),
        range,
    })
}

fn hover_module_qualifier_in_doc(doc: &Document, offset: usize) -> Option<Hover> {
    let ident = qualified_ident_parts_at_offset(&doc.text, offset)?;
    let module = parser::parse_module(&doc.text).ok()?;

    if let Some(module_name) = module.name.as_ref() {
        let module_segments: Vec<_> = module_name.split('.').collect();
        if ident.full_name == *module_name && ident.segment_index < module_segments.len() {
            return Some(simple_hover(
                doc,
                ident.current_span,
                &format!("module {module_name}"),
            ));
        }
    }

    for item in &module.items {
        let ast::Item::Import(import) = item else {
            continue;
        };

        let local = import.as_name.as_deref().unwrap_or(&import.module);
        let local_segments: Vec<_> = local.split('.').collect();
        let matches_exact = ident.full_name == local && ident.segment_index < local_segments.len();
        let matches_qualifier = ident.segments.len() > local_segments.len()
            && ident.segment_index < local_segments.len()
            && ident.segments[..local_segments.len()].join(".") == local;

        if matches_exact || matches_qualifier {
            let head = if import.as_name.is_some() {
                format!("module alias {local} = {}", import.module)
            } else {
                format!("module {}", import.module)
            };
            return Some(simple_hover(doc, ident.current_span, &head));
        }
    }

    None
}

fn hover_parameter_in_doc(
    doc: &Document,
    offset: usize,
    name: &str,
    typed: Option<&types::TypedModule>,
) -> Option<Hover> {
    let module = parser::parse_module(&doc.text).ok()?;
    let tokens = lexer::lex(&doc.text).ok()?;

    for binding in bindings_in_module(&module) {
        let ast::PatternKind::Var(binding_name) = &binding.pat.kind else {
            continue;
        };
        let scheme = typed.and_then(|typed| binding_scheme(typed, binding_name));

        let Some(name_span) = binding_name_span(doc, &tokens, binding) else {
            continue;
        };

        let mut spans = binding_lhs_parameter_spans(doc, &tokens, binding, name_span);
        spans.extend(leading_lambda_parameter_spans(&tokens, binding));

        for (index, span) in spans.into_iter().enumerate() {
            if span.start <= offset && offset < span.end {
                let range = span_to_range(doc, span);
                let value = scheme
                    .and_then(|scheme| function_argument_types(&scheme.ty, index + 1).into_iter().nth(index))
                    .map(|ty| format!("```kscr\nparameter {name} :: {}\n```", types::format_pretty_ty(&ty)))
                    .or_else(|| class_method_parameter_type_text(&module, binding_name, index).map(|ty| format!("```kscr\nparameter {name} :: {ty}\n```")))
                    .unwrap_or_else(|| format!("```kscr\nparameter {name}\n```"));
                return Some(Hover {
                    contents: HoverContents::Markup(MarkupContent {
                        kind: MarkupKind::Markdown,
                        value,
                    }),
                    range,
                });
            }
        }
    }

    None
}

fn hover_parameter_use_in_doc(
    doc: &Document,
    offset: usize,
    typed: Option<&types::TypedModule>,
) -> Option<Hover> {
    let module = parser::parse_module(&doc.text).ok()?;
    let tokens = lexer::lex(&doc.text).ok()?;
    let ident = qualified_ident_parts_at_offset(&doc.text, offset)?;
    if ident.segments.len() != 1 {
        return None;
    }
    let (cursor_line, _) = doc.offset_to_position(offset)?;

    for binding in bindings_in_module(&module) {
        let ast::PatternKind::Var(binding_name) = &binding.pat.kind else {
            continue;
        };
        let scheme = typed.and_then(|typed| binding_scheme(typed, binding_name));
        let Some(name_span) = binding_name_span(doc, &tokens, binding) else {
            continue;
        };
        let (binding_line, _) = doc.offset_to_position(binding.pat.span.start)?;

        let mut spans = binding_lhs_parameter_spans(doc, &tokens, binding, name_span);
        spans.extend(leading_lambda_parameter_spans(&tokens, binding));

        for (index, span) in spans.into_iter().enumerate() {
            if span.start <= offset && offset < span.end {
                return None;
            }

            let Some(name) = doc.text.get(span.start..span.end) else {
                continue;
            };
            let same_line_rhs_use = cursor_line == binding_line && offset >= span.end;
            if name == ident.current_name
                && (expr_contains_var_at_offset(&binding.expr, name, offset) || same_line_rhs_use)
            {
                let head = scheme
                    .and_then(|scheme| function_argument_types(&scheme.ty, index + 1).into_iter().nth(index))
                    .map(|ty| format!("parameter {name} :: {}", types::format_pretty_ty(&ty)))
                    .or_else(|| class_method_parameter_type_text(&module, binding_name, index).map(|ty| format!("parameter {name} :: {ty}")))
                    .unwrap_or_else(|| format!("parameter {name}"));
                return Some(simple_hover(
                    doc,
                    ident.current_span,
                    &head,
                ));
            }
        }
    }

    None
}

fn expr_contains_var_at_offset(expr: &ast::Expr, name: &str, offset: usize) -> bool {
    match &expr.kind {
        ast::ExprKind::Var(var_name) => var_name == name && expr.span.start <= offset && offset < expr.span.end,
        ast::ExprKind::Lambda { body, .. } | ast::ExprKind::Annot { expr: body, .. } => {
            expr_contains_var_at_offset(body, name, offset)
        }
        ast::ExprKind::Apply { func, args } => {
            expr_contains_var_at_offset(func, name, offset)
                || args.iter().any(|arg| expr_contains_var_at_offset(arg, name, offset))
        }
        ast::ExprKind::If {
            cond,
            then_branch,
            else_branch,
        } => {
            expr_contains_var_at_offset(cond, name, offset)
                || expr_contains_var_at_offset(then_branch, name, offset)
                || expr_contains_var_at_offset(else_branch, name, offset)
        }
        ast::ExprKind::Let { bindings, body } => {
            bindings.iter().any(|binding| expr_contains_var_at_offset(&binding.expr, name, offset))
                || expr_contains_var_at_offset(body, name, offset)
        }
        ast::ExprKind::Where { expr, bindings } => {
            expr_contains_var_at_offset(expr, name, offset)
                || bindings.iter().any(|binding| expr_contains_var_at_offset(&binding.expr, name, offset))
        }
        ast::ExprKind::Do(stmts) => stmts.iter().any(|stmt| match stmt {
            ast::DoStmt::Bind { expr, .. } | ast::DoStmt::Expr(expr) => expr_contains_var_at_offset(expr, name, offset),
        }),
        ast::ExprKind::Case { expr, arms } => {
            expr_contains_var_at_offset(expr, name, offset)
                || arms.iter().any(|arm| {
                    arm.guard
                        .as_ref()
                        .is_some_and(|guard| expr_contains_var_at_offset(guard, name, offset))
                        || expr_contains_var_at_offset(&arm.body, name, offset)
                })
        }
        ast::ExprKind::Cons { head, tail } => {
            expr_contains_var_at_offset(head, name, offset)
                || expr_contains_var_at_offset(tail, name, offset)
        }
        ast::ExprKind::List(items) | ast::ExprKind::Tuple(items) => {
            items.iter().any(|item| expr_contains_var_at_offset(item, name, offset))
        }
        ast::ExprKind::Record(fields) => fields
            .iter()
            .any(|(_, value)| expr_contains_var_at_offset(value, name, offset)),
        ast::ExprKind::Unit
        | ast::ExprKind::Integer(_)
        | ast::ExprKind::Float64(_)
        | ast::ExprKind::Bool(_)
        | ast::ExprKind::String(_)
        | ast::ExprKind::Char(_)
        | ast::ExprKind::Ctor(_) => false,
    }
}

fn hover_local_binding_in_doc(
    doc: &Document,
    offset: usize,
    name: &str,
    typed: Option<&types::TypedModule>,
) -> Option<Hover> {
    let typed = typed?;
    let ident = qualified_ident_parts_at_offset(&doc.text, offset)?;
    if ident.segments.len() != 1 {
        return None;
    }

    let module = parser::parse_module(&doc.text).ok()?;
    for item in &module.items {
        let ast::Item::Binding(binding) = item else {
            continue;
        };
        if let Some(hover) = hover_local_binding_in_expr(doc, offset, name, typed, &binding.expr, &[]) {
            return Some(hover);
        }
    }

    None
}

pub(super) fn hover_method_parameter_use_in_doc(
    doc: &Document,
    offset: usize,
    typed: Option<&types::TypedModule>,
) -> Option<Hover> {
    let module = parser::parse_module(&doc.text).ok()?;
    let tokens = lexer::lex(&doc.text).ok()?;
    let ident = qualified_ident_parts_at_offset(&doc.text, offset)?;
    if ident.segments.len() != 1 {
        return None;
    }

    let (cursor_line, _) = doc.offset_to_position(offset)?;
    for item in &module.items {
        let bindings: &[ast::Binding] = match item {
            ast::Item::ClassDecl(class) => &class.default_methods,
            ast::Item::InstanceDecl(inst) => &inst.methods,
            _ => continue,
        };

        for binding in bindings {
            let ast::PatternKind::Var(binding_name) = &binding.pat.kind else {
                continue;
            };
            let scheme = typed.and_then(|typed| binding_scheme(typed, binding_name));
            let Some(name_span) = binding_name_span(doc, &tokens, binding) else {
                continue;
            };
            let (binding_line, _) = doc.offset_to_position(binding.pat.span.start)?;
            if cursor_line != binding_line {
                continue;
            }

            let spans = binding_lhs_parameter_spans(doc, &tokens, binding, name_span);
            for (index, span) in spans.into_iter().enumerate() {
                if span.start <= offset && offset < span.end {
                    return None;
                }
                let Some(name) = doc.text.get(span.start..span.end) else {
                    continue;
                };
                if name == ident.current_name && offset >= span.end {
                    let head = scheme
                        .and_then(|scheme| function_argument_types(&scheme.ty, index + 1).into_iter().nth(index))
                        .map(|ty| format!("parameter {name} :: {}", types::format_pretty_ty(&ty)))
                        .or_else(|| class_method_parameter_type_text(&module, binding_name, index).map(|ty| format!("parameter {name} :: {ty}")))
                        .unwrap_or_else(|| format!("parameter {name}"));
                    return Some(simple_hover(doc, ident.current_span, &head));
                }
            }
        }
    }

    None
}

fn hover_line_parameter_use_in_doc(
    doc: &Document,
    offset: usize,
    typed: Option<&types::TypedModule>,
) -> Option<Hover> {
    let module = parser::parse_module(&doc.text).ok()?;
    let tokens = lexer::lex(&doc.text).ok()?;
    let ident = qualified_ident_parts_at_offset(&doc.text, offset)?;
    if ident.segments.len() != 1 {
        return None;
    }

    let (cursor_line, _) = doc.offset_to_position(offset)?;
    let clause = line_function_clause(doc, &tokens, cursor_line)?;
    if offset < clause.eq_span.end {
        return None;
    }

    for (index, span) in clause.param_spans.iter().copied().enumerate() {
        if span.start <= offset && offset < span.end {
            return None;
        }
        let Some(name) = doc.text.get(span.start..span.end) else {
            continue;
        };
        if name != ident.current_name || offset < span.end {
            continue;
        }

        let scheme = typed.and_then(|typed| binding_scheme(typed, &clause.binding_name));
        let head = scheme
            .and_then(|scheme| function_argument_types(&scheme.ty, index + 1).into_iter().nth(index))
            .map(|ty| format!("parameter {name} :: {}", types::format_pretty_ty(&ty)))
            .or_else(|| {
                class_method_parameter_type_text(&module, &clause.binding_name, index)
                    .map(|ty| format!("parameter {name} :: {ty}"))
            })
            .unwrap_or_else(|| format!("parameter {name}"));
        return Some(simple_hover(doc, ident.current_span, &head));
    }

    None
}

struct LineFunctionClause {
    binding_name: String,
    eq_span: lexer::Span,
    param_spans: Vec<lexer::Span>,
}

fn line_function_clause(
    doc: &Document,
    tokens: &[lexer::Token],
    target_line: u32,
) -> Option<LineFunctionClause> {
    let line_tokens: Vec<&lexer::Token> = tokens
        .iter()
        .filter(|token| {
            doc.offset_to_position(token.span.start)
                .is_some_and(|(line, _)| line == target_line)
        })
        .collect();

    let eq_index = line_tokens
        .iter()
        .position(|token| token.kind == lexer::TokenKind::Eq)?;
    let before_eq = &line_tokens[..eq_index];
    let eq_span = line_tokens[eq_index].span;

    let mut binding_name = None;
    let mut param_spans = Vec::new();
    let mut saw_lparen = false;
    let mut saw_name = false;

    for token in before_eq {
        match &token.kind {
            lexer::TokenKind::Newline | lexer::TokenKind::Indent | lexer::TokenKind::Dedent => {}
            lexer::TokenKind::LParen if !saw_name => saw_lparen = true,
            kind if saw_lparen && is_operator_name_token(kind) && !saw_name => {
                binding_name = Some(token_text(doc, token.span)?.to_string());
                saw_name = true;
            }
            lexer::TokenKind::Ident(_) if !saw_name => {
                binding_name = Some(token_text(doc, token.span)?.to_string());
                saw_name = true;
            }
            lexer::TokenKind::Ident(_) if saw_name => param_spans.push(token.span),
            lexer::TokenKind::RParen => {}
            lexer::TokenKind::ColonColon => return None,
            _ => break,
        }
    }

    Some(LineFunctionClause {
        binding_name: binding_name?,
        eq_span,
        param_spans,
    })
}

fn token_text(doc: &Document, span: lexer::Span) -> Option<&str> {
    doc.text.get(span.start..span.end)
}

fn hover_local_binding_in_expr<'a>(
    doc: &Document,
    offset: usize,
    name: &str,
    typed: &'a types::TypedModule,
    expr: &'a ast::Expr,
    scope: &[&'a ast::Binding],
) -> Option<Hover> {
    if offset < expr.span.start || expr.span.end <= offset {
        return None;
    }

    match &expr.kind {
        ast::ExprKind::Var(var_name) if var_name == name => {
            for binding in scope.iter().rev() {
                let ast::PatternKind::Var(binding_name) = &binding.pat.kind else {
                    continue;
                };
                if binding_name != name {
                    continue;
                }
                let scheme = local_binding_scheme(typed, binding_name, binding.pat.span)?;
                return Some(simple_hover(
                    doc,
                    expr.span,
                    &format!("binding {name} :: {}", types::format_pretty_scheme(scheme)),
                ));
            }
            None
        }
        ast::ExprKind::Var(_) => None,
        ast::ExprKind::Let { bindings, body } => hover_local_binding_in_scope(
            doc, offset, name, typed, bindings, body, scope,
        ),
        ast::ExprKind::Where { expr, bindings } => hover_local_binding_in_scope(
            doc, offset, name, typed, bindings, expr, scope,
        ),
        ast::ExprKind::Lambda { body, .. } | ast::ExprKind::Annot { expr: body, .. } => {
            hover_local_binding_in_expr(doc, offset, name, typed, body, scope)
        }
        ast::ExprKind::Apply { func, args } => hover_local_binding_in_expr(doc, offset, name, typed, func, scope)
            .or_else(|| args.iter().find_map(|arg| hover_local_binding_in_expr(doc, offset, name, typed, arg, scope))),
        ast::ExprKind::If {
            cond,
            then_branch,
            else_branch,
        } => hover_local_binding_in_expr(doc, offset, name, typed, cond, scope)
            .or_else(|| hover_local_binding_in_expr(doc, offset, name, typed, then_branch, scope))
            .or_else(|| hover_local_binding_in_expr(doc, offset, name, typed, else_branch, scope)),
        ast::ExprKind::Do(stmts) => stmts.iter().find_map(|stmt| match stmt {
            ast::DoStmt::Bind { expr, .. } | ast::DoStmt::Expr(expr) => {
                hover_local_binding_in_expr(doc, offset, name, typed, expr, scope)
            }
        }),
        ast::ExprKind::Case { expr, arms } => hover_local_binding_in_expr(doc, offset, name, typed, expr, scope)
            .or_else(|| {
                arms.iter().find_map(|arm| {
                    arm.guard
                        .as_ref()
                        .and_then(|guard| hover_local_binding_in_expr(doc, offset, name, typed, guard, scope))
                        .or_else(|| hover_local_binding_in_expr(doc, offset, name, typed, &arm.body, scope))
                })
            }),
        ast::ExprKind::Cons { head, tail } => hover_local_binding_in_expr(doc, offset, name, typed, head, scope)
            .or_else(|| hover_local_binding_in_expr(doc, offset, name, typed, tail, scope)),
        ast::ExprKind::List(items) | ast::ExprKind::Tuple(items) => {
            items.iter().find_map(|item| hover_local_binding_in_expr(doc, offset, name, typed, item, scope))
        }
        ast::ExprKind::Record(fields) => fields
            .iter()
            .find_map(|(_, value)| hover_local_binding_in_expr(doc, offset, name, typed, value, scope)),
        ast::ExprKind::Unit
        | ast::ExprKind::Integer(_)
        | ast::ExprKind::Float64(_)
        | ast::ExprKind::Bool(_)
        | ast::ExprKind::String(_)
        | ast::ExprKind::Char(_)
        | ast::ExprKind::Ctor(_) => None,
    }
}

fn hover_local_binding_in_scope<'a>(
    doc: &Document,
    offset: usize,
    name: &str,
    typed: &'a types::TypedModule,
    bindings: &'a [ast::Binding],
    body: &'a ast::Expr,
    scope: &[&'a ast::Binding],
) -> Option<Hover> {
    for binding in bindings {
        let ast::PatternKind::Var(binding_name) = &binding.pat.kind else {
            continue;
        };
        if binding_name == name && binding.pat.span.start <= offset && offset < binding.pat.span.end {
            let scheme = local_binding_scheme(typed, binding_name, binding.pat.span)?;
            return Some(simple_hover(
                doc,
                binding.pat.span,
                &format!("binding {name} :: {}", types::format_pretty_scheme(scheme)),
            ));
        }
    }

    let mut scoped_bindings: Vec<&ast::Binding> = scope.to_vec();
    scoped_bindings.extend(bindings.iter());
    for binding in bindings {
        if let Some(hover) = hover_local_binding_in_expr(doc, offset, name, typed, &binding.expr, &scoped_bindings) {
            return Some(hover);
        }
    }
    hover_local_binding_in_expr(doc, offset, name, typed, body, &scoped_bindings)
}

fn local_binding_scheme<'a>(
    typed: &'a types::TypedModule,
    name: &str,
    span: lexer::Span,
) -> Option<&'a types::Scheme> {
    typed
        .local_bindings
        .iter()
        .rev()
        .find(|binding| binding.name == name && binding.span == span)
        .map(|binding| &binding.scheme)
}

fn bindings_in_module(module: &ast::Module) -> Vec<&ast::Binding> {
    let mut out = Vec::new();
    for item in &module.items {
        match item {
            ast::Item::Binding(binding) => {
                out.push(binding);
                collect_nested_bindings(&binding.expr, &mut out);
            }
            ast::Item::ClassDecl(class) => {
                for binding in &class.default_methods {
                    out.push(binding);
                    collect_nested_bindings(&binding.expr, &mut out);
                }
            }
            ast::Item::InstanceDecl(inst) => {
                for binding in &inst.methods {
                    out.push(binding);
                    collect_nested_bindings(&binding.expr, &mut out);
                }
            }
            ast::Item::Import(_)
            | ast::Item::Export(_)
            | ast::Item::Fixity(_)
            | ast::Item::TypeAlias(_)
            | ast::Item::DataDecl(_) => {}
        }
    }
    out
}

fn binding_scheme<'a>(typed: &'a types::TypedModule, binding_name: &str) -> Option<&'a types::Scheme> {
    typed
        .inferred
        .get(binding_name)
        .or_else(|| typed.class_methods.get(binding_name))
}

fn class_method_parameter_type_text(
    module: &ast::Module,
    binding_name: &str,
    index: usize,
) -> Option<String> {
    module.items.iter().find_map(|item| {
        let ast::Item::ClassDecl(class) = item else {
            return None;
        };
        class
            .methods
            .iter()
            .find(|method| method.name == binding_name)
            .and_then(|method| ast_function_argument_types(&method.ty.ty, index + 1).into_iter().nth(index))
            .map(format_ast_type)
    })
}

fn ast_function_argument_types<'a>(ty: &'a ast::Type, count: usize) -> Vec<&'a ast::Type> {
    let mut out = Vec::new();
    let mut current = ty;
    while out.len() < count {
        let ast::Type::Func(arg, rest) = current else {
            break;
        };
        out.push(arg.as_ref());
        current = rest;
    }
    out
}

fn format_ast_type(ty: &ast::Type) -> String {
    match ty {
        ast::Type::Unit => "()".to_string(),
        ast::Type::Integer => "Integer".to_string(),
        ast::Type::Bool => "Bool".to_string(),
        ast::Type::Float64 => "Float64".to_string(),
        ast::Type::Char => "Char".to_string(),
        ast::Type::String => "String".to_string(),
        ast::Type::List(item) => format!("[{}]", format_ast_type(item)),
        ast::Type::Tuple(items) => format!("({})", items.iter().map(format_ast_type).collect::<Vec<_>>().join(", ")),
        ast::Type::Record(fields) => format!("{{{}}}", fields.iter().map(|(name, ty)| format!("{name}: {}", format_ast_type(ty))).collect::<Vec<_>>().join(", ")),
        ast::Type::RecordOpen(fields, rest) => format!("{{{}, ..{}}}", fields.iter().map(|(name, ty)| format!("{name}: {}", format_ast_type(ty))).collect::<Vec<_>>().join(", "), format_ast_type(rest)),
        ast::Type::Hole(name) => name.clone().unwrap_or_else(|| "_".to_string()),
        ast::Type::Var(name) => name.clone(),
        ast::Type::App { head, args } => {
            let mut parts = vec![format_ast_type(head)];
            parts.extend(args.iter().map(format_ast_type));
            parts.join(" ")
        }
        ast::Type::Func(arg, rest) => format!("{} -> {}", format_ast_type(arg), format_ast_type(rest)),
    }
}

fn collect_nested_bindings<'a>(expr: &'a ast::Expr, out: &mut Vec<&'a ast::Binding>) {
    match &expr.kind {
        ast::ExprKind::Lambda { body, .. } | ast::ExprKind::Annot { expr: body, .. } => {
            collect_nested_bindings(body, out);
        }
        ast::ExprKind::Apply { func, args } => {
            collect_nested_bindings(func, out);
            for arg in args {
                collect_nested_bindings(arg, out);
            }
        }
        ast::ExprKind::If {
            cond,
            then_branch,
            else_branch,
        } => {
            collect_nested_bindings(cond, out);
            collect_nested_bindings(then_branch, out);
            collect_nested_bindings(else_branch, out);
        }
        ast::ExprKind::Let { bindings, body } => {
            for binding in bindings {
                out.push(binding);
                collect_nested_bindings(&binding.expr, out);
            }
            collect_nested_bindings(body, out);
        }
        ast::ExprKind::Where { expr, bindings } => {
            collect_nested_bindings(expr, out);
            for binding in bindings {
                out.push(binding);
                collect_nested_bindings(&binding.expr, out);
            }
        }
        ast::ExprKind::Do(stmts) => {
            for stmt in stmts {
                match stmt {
                    ast::DoStmt::Bind { expr, .. } | ast::DoStmt::Expr(expr) => {
                        collect_nested_bindings(expr, out);
                    }
                }
            }
        }
        ast::ExprKind::Case { expr, arms } => {
            collect_nested_bindings(expr, out);
            for arm in arms {
                if let Some(guard) = &arm.guard {
                    collect_nested_bindings(guard, out);
                }
                collect_nested_bindings(&arm.body, out);
            }
        }
        ast::ExprKind::Cons { head, tail } => {
            collect_nested_bindings(head, out);
            collect_nested_bindings(tail, out);
        }
        ast::ExprKind::List(items) | ast::ExprKind::Tuple(items) => {
            for item in items {
                collect_nested_bindings(item, out);
            }
        }
        ast::ExprKind::Record(fields) => {
            for (_, expr) in fields {
                collect_nested_bindings(expr, out);
            }
        }
        ast::ExprKind::Unit
        | ast::ExprKind::Integer(_)
        | ast::ExprKind::Float64(_)
        | ast::ExprKind::Bool(_)
        | ast::ExprKind::String(_)
        | ast::ExprKind::Char(_)
        | ast::ExprKind::Var(_)
        | ast::ExprKind::Ctor(_) => {}
    }
}

fn simple_hover(doc: &Document, span: lexer::Span, head: &str) -> Hover {
    Hover {
        contents: HoverContents::Markup(MarkupContent {
            kind: MarkupKind::Markdown,
            value: format!("```kscr\n{head}\n```"),
        }),
        range: span_to_range(doc, span),
    }
}

pub(super) fn function_argument_types(ty: &types::Ty, count: usize) -> Vec<types::Ty> {
    let mut out = Vec::new();
    let mut current = ty;
    while out.len() < count {
        let types::Ty::Func(arg, rest) = current else {
            break;
        };
        out.push((**arg).clone());
        current = rest;
    }
    out
}

pub(super) fn binding_lhs_parameter_spans(
    doc: &Document,
    tokens: &[lexer::Token],
    binding: &kscr::ast::Binding,
    name_span: kscr::lexer::Span,
) -> Vec<kscr::lexer::Span> {
    let Some((binding_line, _)) = doc.offset_to_position(binding.pat.span.start) else {
        return Vec::new();
    };

    let mut out = Vec::new();
    for token in tokens {
        if token.span.start <= name_span.start {
            continue;
        }

        let Some((line, _)) = doc.offset_to_position(token.span.start) else {
            continue;
        };
        if line != binding_line {
            break;
        }

        match &token.kind {
            lexer::TokenKind::Ident(_) => out.push(token.span),
            lexer::TokenKind::Eq => break,
            lexer::TokenKind::ColonColon => return Vec::new(),
            _ => {}
        }
    }
    out
}

fn is_operator_name_token(kind: &lexer::TokenKind) -> bool {
    matches!(
        kind,
        lexer::TokenKind::Operator(_)
            | lexer::TokenKind::Colon
            | lexer::TokenKind::Plus
            | lexer::TokenKind::Minus
            | lexer::TokenKind::Star
            | lexer::TokenKind::Slash
            | lexer::TokenKind::PlusPlus
            | lexer::TokenKind::EqEq
            | lexer::TokenKind::SlashEq
            | lexer::TokenKind::Lt
            | lexer::TokenKind::Le
            | lexer::TokenKind::Gt
            | lexer::TokenKind::Ge
            | lexer::TokenKind::GtGt
            | lexer::TokenKind::GtGtEq
            | lexer::TokenKind::AndAnd
            | lexer::TokenKind::OrOr
    )
}

pub(super) fn binding_name_span(
    doc: &Document,
    tokens: &[lexer::Token],
    binding: &kscr::ast::Binding,
) -> Option<kscr::lexer::Span> {
    let (binding_line, _) = doc.offset_to_position(binding.pat.span.start)?;

    let mut saw_lparen = false;
    for token in tokens {
        if token.span.start < binding.pat.span.start {
            continue;
        }

        let (line, _) = doc.offset_to_position(token.span.start)?;
        if line != binding_line {
            break;
        }

        match &token.kind {
            lexer::TokenKind::Ident(_) => return Some(token.span),
            kind if saw_lparen && is_operator_name_token(kind) => return Some(token.span),
            lexer::TokenKind::LParen => saw_lparen = true,
            lexer::TokenKind::Eq | lexer::TokenKind::ColonColon => break,
            lexer::TokenKind::Indent => {}
            _ => break,
        }
    }

    None
}

pub(super) fn leading_lambda_parameter_spans(
    tokens: &[lexer::Token],
    binding: &kscr::ast::Binding,
) -> Vec<kscr::lexer::Span> {
    if !matches!(binding.expr.kind, kscr::ast::ExprKind::Lambda { .. }) {
        return Vec::new();
    }

    let mut in_expr = false;
    let mut out = Vec::new();
    for token in tokens {
        if token.span.start < binding.expr.span.start {
            continue;
        }
        if token.span.start >= binding.expr.span.end {
            break;
        }

        if !in_expr {
            if token.kind == lexer::TokenKind::Backslash {
                in_expr = true;
            }
            continue;
        }

        match &token.kind {
            lexer::TokenKind::Ident(_) => out.push(token.span),
            lexer::TokenKind::Arrow => break,
            lexer::TokenKind::Newline | lexer::TokenKind::Indent | lexer::TokenKind::Dedent => {}
            _ => break,
        }
    }
    out
}
