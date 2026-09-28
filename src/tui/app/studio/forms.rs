//! The builder's forms: one for a template's own fields, one for its ID, one
//! per variable, and what each hands back.

use super::*;

// ---------------------------------------------------------------------------
// The forms each section is answered in
// ---------------------------------------------------------------------------

pub fn metadata_form(template: &Template) -> Form {
    // **On an edit the slug is already chosen, so it stops following the
    // name.** `suggest_slug` rewrites the slug of any field nobody has typed
    // in, and a form built from an existing template has touched nothing — so
    // without the flag, correcting a typo in the title retypes the slug, and
    // Save renames the template's directory on disk to match. The flag says
    // "this value was chosen", which for a loaded template it was.
    let mut slug = Field::text(
        "slug",
        "Slug",
        "its folder name and its command-line argument — lowercase, no spaces",
        template.slug.clone(),
    );
    slug.touched = !template.slug.is_empty();

    Form::new(vec![
        Field::text(
            "name",
            "Name",
            "what the template is called",
            template.name.clone(),
        ),
        slug,
        Field::text(
            "description",
            "Description",
            "one line, shown in the list and the picker (optional)",
            template.description.clone(),
        ),
        Field::text(
            "naming_pattern",
            "Naming pattern",
            "tokens: {date} {YYYY} {MM} {DD} {id} and any variable slug",
            if template.naming_pattern.is_empty() {
                "{date}_{id}".to_string()
            } else {
                template.naming_pattern.clone()
            },
        ),
    ])
}

pub fn id_form(template: &Template) -> Form {
    Form::new(vec![
        Field::text(
            "prefix",
            "Prefix",
            "register recovers an ID by matching <prefix><digits> in a folder name",
            template.id.prefix.clone(),
        ),
        Field::text(
            "digits",
            "Digits",
            "zero-padded width — how wide ID0001 is",
            template.id.digits.to_string(),
        ),
    ])
}

/// The transform names, in the order the form cycles through them.
pub const TRANSFORMS: [&str; 4] = [
    "none",
    "TitleUnderscore",
    "UpperUnderscore",
    "LowerUnderscore",
];

pub fn transform_of(label: &str) -> Transform {
    match label {
        "TitleUnderscore" => Transform::TitleUnderscore,
        "UpperUnderscore" => Transform::UpperUnderscore,
        "LowerUnderscore" => Transform::LowerUnderscore,
        _ => Transform::None,
    }
}

pub fn transform_label(transform: Transform) -> &'static str {
    match transform {
        Transform::None => "none",
        Transform::TitleUnderscore => "TitleUnderscore",
        Transform::UpperUnderscore => "UpperUnderscore",
        Transform::LowerUnderscore => "LowerUnderscore",
    }
}

/// One variable's form. `None` builds an empty one.
///
/// A select's options are one comma-separated line: on a form the whole
/// answer has to be visible and correctable, and a list of three words is a
/// line.
pub fn variable_form(existing: Option<&Variable>) -> Form {
    let blank = Variable {
        slug: String::new(),
        label: String::new(),
        var_type: VarType::Text,
        required: false,
        options: Vec::new(),
        default: String::new(),
        transform: Transform::None,
    };
    let v = existing.cloned().unwrap_or(blank);
    let type_at = usize::from(v.var_type == VarType::Select);
    let transform_at = TRANSFORMS
        .iter()
        .position(|label| *label == transform_label(v.transform))
        .unwrap_or(0);
    Form::new(vec![
        Field::text("slug", "Slug", "the token: {artist}", v.slug.clone()),
        Field::text("label", "Label", "what the question says", v.label.clone()),
        Field::choice(
            "type",
            "Type",
            "text is typed; select is picked from the options below",
            vec!["text".to_string(), "select".to_string()],
            type_at,
        ),
        Field::text(
            "options",
            "Options",
            "for a select: the answers, separated by commas",
            v.options.join(", "),
        )
        .hidden(v.var_type != VarType::Select),
        Field::text(
            "default",
            "Default",
            "offered as the answer; Enter keeps it (optional)",
            v.default.clone(),
        ),
        Field::choice(
            "transform",
            "Transform",
            "how the answer is reshaped for the folder name",
            TRANSFORMS.iter().map(|t| (*t).to_string()).collect(),
            transform_at,
        ),
        Field::toggle(
            "required",
            "Required",
            "a required variable cannot be left empty",
            v.required,
        ),
    ])
}

/// Show the options line only for a select — a text variable has none — and
/// let the transform row show what it would do to an answer.
pub fn sync_variable_form(form: &mut Form, g: Glyphs) {
    let is_select = form.value("type") == "select";
    form.set_hidden("options", !is_select);

    let shown = transform_example(&form.value("transform"), g);
    if let Some(field) = form.field_mut("transform") {
        field.hint = shown;
    }
}

/// What a transform does, said with an answer rather than a name.
///
/// `TitleUnderscore` is the manifest's word and stays on the row — it is what
/// the YAML says — but nothing about it tells you that a space becomes an
/// underscore, and the four names differ from each other only in ways you have
/// to already know to read.
pub fn transform_example(label: &str, g: Glyphs) -> String {
    let after = match label {
        "TitleUnderscore" => "Ariana_Grande",
        "UpperUnderscore" => "ARIANA_GRANDE",
        "LowerUnderscore" => "ariana_grande",
        _ => "Ariana Grande (left exactly as typed)",
    };
    format!(
        "how the answer is reshaped for the folder name: Ariana Grande {} {after}",
        g.arrow
    )
}

/// Keep the metadata form's own advice current as it is typed: the slug
/// follows the name until one is typed, and the naming-pattern row says what
/// the pattern would actually do with the variables this template declares.
pub fn sync_metadata_form(form: &mut Form, declared: &[&str]) {
    suggest_slug(form);
    let pattern = form.value("naming_pattern");
    let hint = pattern_warning_of(&pattern, declared).unwrap_or_else(|| {
        let built_ins = BUILT_IN_TOKENS
            .iter()
            .map(|token| format!("{{{token}}}"))
            .collect::<Vec<_>>()
            .join(" ");
        match declared.is_empty() {
            true => format!("tokens: {built_ins}"),
            false => format!(
                "tokens: {built_ins} and {}",
                declared
                    .iter()
                    .map(|s| format!("{{{s}}}"))
                    .collect::<Vec<_>>()
                    .join(" ")
            ),
        }
    });
    if let Some(field) = form.field_mut("naming_pattern") {
        field.hint = hint;
    }
}

/// The variable a form describes, or the message refusing it.
pub fn variable_from(form: &Form) -> Result<Variable, (&'static str, String)> {
    let slug = form.value("slug").trim().to_string();
    if slug.is_empty() {
        return Err(("slug", "a variable needs a slug — it is the token".into()));
    }
    let var_type = if form.value("type") == "select" {
        VarType::Select
    } else {
        VarType::Text
    };
    let options: Vec<String> = form
        .value("options")
        .split(',')
        .map(|option| option.trim().to_string())
        .filter(|option| !option.is_empty())
        .collect();
    if var_type == VarType::Select && options.is_empty() {
        return Err((
            "options",
            "a select variable needs at least one option".into(),
        ));
    }
    let label = form.value("label");
    Ok(Variable {
        label: if label.trim().is_empty() {
            slug.clone()
        } else {
            label
        },
        slug,
        var_type,
        required: form.is_on("required"),
        options,
        default: form.value("default"),
        transform: transform_of(&form.value("transform")),
    })
}

/// The file a path and a body describe, or the message refusing it.
pub fn file_from(edit: &FileEdit) -> Result<FileEntry, String> {
    let path = edit.path.text().trim().to_string();
    if path.is_empty() {
        return Err("a file needs a path".to_string());
    }
    if crate::core::project_info::path_is_reserved(&path) {
        return Err(format!(
            "'{path}' is reserved by fastf — every new project gets one automatically"
        ));
    }
    let body = edit.body.text();
    Ok(FileEntry {
        path,
        // Always stored as a template: interpolation is a no-op on text with no
        // braces, so there is nothing to lose and `{slug}` markers just work.
        template: if body.is_empty() {
            String::new()
        } else if body.ends_with('\n') {
            body
        } else {
            format!("{body}\n")
        },
        content: String::new(),
    })
}
