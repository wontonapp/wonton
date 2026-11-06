use std::collections::{BTreeMap, BTreeSet};

use fluent_bundle::FluentResource;
use fluent_syntax::ast;
use unic_langid::LanguageIdentifier;

type Variables = BTreeSet<String>;
type Schema = BTreeMap<String, Variables>;

pub fn check_catalog(resources: &[(&str, &str)]) -> Result<(), Vec<String>> {
    let mut errors = Vec::new();
    if resources.first().map(|(tag, _)| *tag) != Some("en") {
        errors.push("the first catalog language must be en".to_owned());
    }
    let mut languages = BTreeSet::new();
    let mut base = None;
    for (tag, text) in resources {
        match tag.parse::<LanguageIdentifier>() {
            Ok(language) => {
                if !languages.insert(language.to_string()) {
                    errors.push(format!("{tag}: duplicate language"));
                }
            }
            Err(error) => errors.push(format!("{tag}: invalid language tag: {error}")),
        }
        let resource = match FluentResource::try_new((*text).to_owned()) {
            Ok(resource) => resource,
            Err((resource, parse_errors)) => {
                errors.extend(parse_errors.iter().map(|error| format!("{tag}: {error:?}")));
                resource
            }
        };
        let schema = schema(&resource, tag, &mut errors);
        if let Some(base) = &base {
            compare(base, &schema, tag, &mut errors);
        } else {
            base = Some(schema);
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

fn schema(resource: &FluentResource, tag: &str, errors: &mut Vec<String>) -> Schema {
    let mut schema = Schema::new();
    for entry in resource.entries() {
        let (id, value, attributes) = match entry {
            ast::Entry::Message(message) => (
                message.id.name.to_owned(),
                message.value.as_ref(),
                &message.attributes,
            ),
            ast::Entry::Term(term) => (
                format!("-{}", term.id.name),
                Some(&term.value),
                &term.attributes,
            ),
            _ => continue,
        };
        let mut variables = Variables::new();
        if let Some(value) = value {
            pattern_variables(value, &mut variables);
        }
        for attribute in attributes {
            pattern_variables(&attribute.value, &mut variables);
        }
        if schema.insert(id.clone(), variables).is_some() {
            errors.push(format!("{tag}: duplicate message or term {id}"));
        }
    }
    schema
}

fn compare(base: &Schema, other: &Schema, tag: &str, errors: &mut Vec<String>) {
    for (id, variables) in base {
        match other.get(id) {
            None => errors.push(format!("{tag}: missing message or term {id}")),
            Some(other) if other != variables => errors.push(format!(
                "{tag}: {id} variables differ: en={variables:?}, {tag}={other:?}"
            )),
            Some(_) => {}
        }
    }
    for id in other.keys().filter(|id| !base.contains_key(*id)) {
        errors.push(format!("{tag}: extra message or term {id}"));
    }
}

fn pattern_variables(pattern: &ast::Pattern<&str>, variables: &mut Variables) {
    for element in &pattern.elements {
        if let ast::PatternElement::Placeable { expression } = element {
            expression_variables(expression, variables);
        }
    }
}

fn expression_variables(expression: &ast::Expression<&str>, variables: &mut Variables) {
    match expression {
        ast::Expression::Inline(inline) => inline_variables(inline, variables),
        ast::Expression::Select { selector, variants } => {
            inline_variables(selector, variables);
            for variant in variants {
                pattern_variables(&variant.value, variables);
            }
        }
    }
}

fn inline_variables(inline: &ast::InlineExpression<&str>, variables: &mut Variables) {
    match inline {
        ast::InlineExpression::VariableReference { id } => {
            variables.insert(id.name.to_owned());
        }
        ast::InlineExpression::FunctionReference { arguments, .. } => {
            argument_variables(arguments, variables);
        }
        ast::InlineExpression::TermReference {
            arguments: Some(arguments),
            ..
        } => {
            argument_variables(arguments, variables);
        }
        ast::InlineExpression::Placeable { expression } => {
            expression_variables(expression, variables);
        }
        _ => {}
    }
}

fn argument_variables(arguments: &ast::CallArguments<&str>, variables: &mut Variables) {
    for argument in &arguments.positional {
        inline_variables(argument, variables);
    }
    for argument in &arguments.named {
        inline_variables(&argument.value, variables);
    }
}
