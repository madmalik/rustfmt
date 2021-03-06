// Copyright 2015 The Rust Project Developers. See the COPYRIGHT
// file at the top-level directory of this distribution and at
// http://rust-lang.org/COPYRIGHT.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

use visitor::FmtVisitor;
use utils::*;
use lists::{write_list, ListFormatting, SeparatorTactic, ListTactic};

use syntax::{ast, ptr};
use syntax::codemap::{Pos, Span};
use syntax::parse::token;
use syntax::print::pprust;

use MIN_STRING;

impl<'a> FmtVisitor<'a> {
    fn rewrite_string_lit(&mut self, s: &str, span: Span, width: usize, offset: usize) -> String {
        // FIXME I bet this stomps unicode escapes in the source string

        // Check if there is anything to fix: we always try to fixup multi-line
        // strings, or if the string is too long for the line.
        let l_loc = self.codemap.lookup_char_pos(span.lo);
        let r_loc = self.codemap.lookup_char_pos(span.hi);
        if l_loc.line == r_loc.line && r_loc.col.to_usize() <= config!(max_width) {
            return self.snippet(span);
        }

        // TODO if lo.col > IDEAL - 10, start a new line (need cur indent for that)

        let s = s.escape_default();

        let offset = offset + 1;
        let indent = make_indent(offset);
        let indent = &indent;

        let mut cur_start = 0;
        let mut result = String::with_capacity(round_up_to_power_of_two(s.len()));
        result.push('"');
        loop {
            let max_chars = if cur_start == 0 {
                // First line.
                width - 2 // 2 = " + \
            } else {
                config!(max_width) - offset - 1 // 1 = either \ or ;
            };

            let mut cur_end = cur_start + max_chars;

            if cur_end >= s.len() {
                result.push_str(&s[cur_start..]);
                break;
            }

            // Make sure we're on a char boundary.
            cur_end = next_char(&s, cur_end);

            // Push cur_end left until we reach whitespace
            while !s.char_at(cur_end-1).is_whitespace() {
                cur_end = prev_char(&s, cur_end);

                if cur_end - cur_start < MIN_STRING {
                    // We can't break at whitespace, fall back to splitting
                    // anywhere that doesn't break an escape sequence
                    cur_end = next_char(&s, cur_start + max_chars);
                    while s.char_at(prev_char(&s, cur_end)) == '\\' {
                        cur_end = prev_char(&s, cur_end);
                    }
                    break;
                }
            }
            // Make sure there is no whitespace to the right of the break.
            while cur_end < s.len() && s.char_at(cur_end).is_whitespace() {
                cur_end = next_char(&s, cur_end+1);
            }
            result.push_str(&s[cur_start..cur_end]);
            result.push_str("\\\n");
            result.push_str(indent);

            cur_start = cur_end;
        }
        result.push('"');

        result
    }

    fn rewrite_call(&mut self,
                    callee: &ast::Expr,
                    args: &[ptr::P<ast::Expr>],
                    width: usize,
                    offset: usize)
        -> String
    {
        debug!("rewrite_call, width: {}, offset: {}", width, offset);

        // TODO using byte lens instead of char lens (and probably all over the place too)
        let callee_str = self.rewrite_expr(callee, width, offset);
        debug!("rewrite_call, callee_str: `{}`", callee_str);
        // 2 is for parens.
        let remaining_width = width - callee_str.len() - 2;
        let offset = callee_str.len() + 1 + offset;
        let arg_count = args.len();

        let args_str = if arg_count > 0 {
            let args: Vec<_> = args.iter().map(|e| (self.rewrite_expr(e,
                                                                      remaining_width,
                                                                      offset), String::new())).collect();
            let fmt = ListFormatting {
                tactic: ListTactic::HorizontalVertical,
                separator: ",",
                trailing_separator: SeparatorTactic::Never,
                indent: offset,
                h_width: remaining_width,
                v_width: remaining_width,
            };
            write_list(&args, &fmt)
        } else {
            String::new()
        };

        format!("{}({})", callee_str, args_str)
    }

    fn rewrite_paren(&mut self, subexpr: &ast::Expr, width: usize, offset: usize) -> String {
        debug!("rewrite_paren, width: {}, offset: {}", width, offset);
        // 1 is for opening paren, 2 is for opening+closing, we want to keep the closing
        // paren on the same line as the subexpr
        let subexpr_str = self.rewrite_expr(subexpr, width-2, offset+1);
        debug!("rewrite_paren, subexpr_str: `{}`", subexpr_str);
        format!("({})", subexpr_str)
    }

    fn rewrite_struct_lit(&mut self,
                          path: &ast::Path,
                          fields: &[ast::Field],
                          base: Option<&ast::Expr>,
                          width: usize,
                          offset: usize)
        -> String
    {
        debug!("rewrite_struct_lit: width {}, offset {}", width, offset);
        assert!(fields.len() > 0 || base.is_some());

        let path_str = pprust::path_to_string(path);
        // Foo { a: Foo } - indent is +3, width is -5.
        let indent = offset + path_str.len() + 3;
        let budget = width - (path_str.len() + 5);

        let mut field_strs: Vec<_> =
            fields.iter().map(|f| self.rewrite_field(f, budget, indent)).collect();
        if let Some(expr) = base {
            // Another 2 on the width/indent for the ..
            field_strs.push(format!("..{}", self.rewrite_expr(expr, budget - 2, indent + 2)))
        }

        // FIXME comments
        let field_strs: Vec<_> = field_strs.into_iter().map(|s| (s, String::new())).collect();
        let fmt = ListFormatting {
            tactic: ListTactic::HorizontalVertical,
            separator: ",",
            trailing_separator: if base.is_some() {
                    SeparatorTactic::Never
                } else {
                    config!(struct_lit_trailing_comma)
                },
            indent: indent,
            h_width: budget,
            v_width: budget,
        };
        let fields_str = write_list(&field_strs, &fmt);
        format!("{} {{ {} }}", path_str, fields_str)

        // FIXME if the usual multi-line layout is too wide, we should fall back to
        // Foo {
        //     a: ...,
        // }
    }

    fn rewrite_field(&mut self, field: &ast::Field, width: usize, offset: usize) -> String {
        let name = &token::get_ident(field.ident.node);
        let overhead = name.len() + 2;
        let expr = self.rewrite_expr(&field.expr, width - overhead, offset + overhead);
        format!("{}: {}", name, expr)
    }

    fn rewrite_tuple_lit(&mut self, items: &[ptr::P<ast::Expr>], width: usize, offset: usize)
        -> String {
        // opening paren
        let indent = offset + 1;
        // In case of length 1, need a trailing comma
        if items.len() == 1 {
            return format!("({},)", self.rewrite_expr(&*items[0], width - 3, indent));
        }
        // Only last line has width-1 as budget, other may take max_width
        let item_strs: Vec<_> =
            items.iter()
                 .enumerate()
                 .map(|(i, item)| self.rewrite_expr(
                    item,
                    // last line : given width (minus "("+")"), other lines : max_width
                    // (minus "("+","))
                    if i == items.len() - 1 { width - 2 } else { config!(max_width) - indent - 2 },
                    indent))
                 .collect();
        let tactics = if item_strs.iter().any(|s| s.contains('\n')) {
            ListTactic::Vertical
        } else {
            ListTactic::HorizontalVertical
        };
        // FIXME handle comments
        let item_strs: Vec<_> = item_strs.into_iter().map(|s| (s, String::new())).collect();
        let fmt = ListFormatting {
            tactic: tactics,
            separator: ",",
            trailing_separator: SeparatorTactic::Never,
            indent: indent,
            h_width: width - 2,
            v_width: width - 2,
        };
        let item_str = write_list(&item_strs, &fmt);
        format!("({})", item_str)
    }


    pub fn rewrite_expr(&mut self, expr: &ast::Expr, width: usize, offset: usize) -> String {
        match expr.node {
            ast::Expr_::ExprLit(ref l) => {
                match l.node {
                    ast::Lit_::LitStr(ref is, _) => {
                        let result = self.rewrite_string_lit(&is, l.span, width, offset);
                        debug!("string lit: `{}`", result);
                        return result;
                    }
                    _ => {}
                }
            }
            ast::Expr_::ExprCall(ref callee, ref args) => {
                return self.rewrite_call(callee, args, width, offset);
            }
            ast::Expr_::ExprParen(ref subexpr) => {
                return self.rewrite_paren(subexpr, width, offset);
            }
            ast::Expr_::ExprStruct(ref path, ref fields, ref base) => {
                return self.rewrite_struct_lit(path,
                                               fields,
                                               base.as_ref().map(|e| &**e),
                                               width,
                                               offset);
            }
            ast::Expr_::ExprTup(ref items) => {
                return self.rewrite_tuple_lit(items, width, offset);
            }
            _ => {}
        }

        self.snippet(expr.span)
    }
}
