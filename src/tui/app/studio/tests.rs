use super::*;

#[test]
fn paths_round_trip_through_the_tree() {
    let paths = vec![
        "01_Assets".to_string(),
        "01_Assets/01_Audio".to_string(),
        "02_Edit".to_string(),
    ];
    let tree = parse_paths_to_tree(&paths);
    assert_eq!(flatten_tree(&tree, ""), paths);
    assert_eq!(tree.len(), 2, "a nested path merges into its parent");
}

#[test]
fn a_nested_path_creates_the_parents_it_names() {
    let tree = parse_paths_to_tree(&["a/b/c".to_string()]);
    assert_eq!(
        flatten_tree(&tree, ""),
        vec!["a".to_string(), "a/b".to_string(), "a/b/c".to_string()]
    );
}

#[test]
fn the_section_summaries_are_what_the_list_shows() {
    let g = Glyphs::unicode();
    let mut builder = Builder::new(None);
    assert_eq!(builder.summary(Section::Variables, g), "(none)");
    assert_eq!(builder.summary(Section::Structure, g), "(none)");
    builder.template.name = "Music video".to_string();
    builder.template.slug = "music-video".to_string();
    builder.template.naming_pattern = "{date}_{id}".to_string();
    assert_eq!(
        builder.summary(Section::Metadata, g),
        "Music video · music-video · {date}_{id}"
    );
    builder.template.structure = parse_paths_to_tree(&["a".into(), "a/b".into()]);
    assert_eq!(builder.summary(Section::Structure, g), "2 folders");
}

/// Every character the list draws comes from the alphabet it was handed,
/// so a console with no `·` or `…` gets the ASCII spellings rather than
/// two replacement boxes.
#[test]
fn the_summaries_draw_in_the_alphabet_they_are_given() {
    let mut builder = Builder::new(None);
    builder.template.name = "Music video".to_string();
    builder.template.slug = "music-video".to_string();
    builder.template.naming_pattern = "{date}_{id}".to_string();
    assert_eq!(
        builder.summary(Section::Metadata, Glyphs::ascii()),
        "Music video - music-video - {date}_{id}"
    );
    assert_eq!(
        builder.summary(Section::Id, Glyphs::ascii()),
        "ID0001, ID0002 ..."
    );
}

#[test]
fn metadata_is_refused_by_the_rules_template_validate_enforces() {
    let mut form = metadata_form(&Template::default());
    assert_eq!(check_metadata(&form).unwrap().0, "name");
    form.field_mut("name").unwrap().set_text("Music video");
    assert_eq!(check_metadata(&form).unwrap().0, "slug");
    form.field_mut("slug").unwrap().set_text("music video");
    assert_eq!(check_metadata(&form).unwrap().0, "slug", "spaces refused");
    form.field_mut("slug").unwrap().set_text("music-video");
    form.field_mut("naming_pattern")
        .unwrap()
        .set_text(".hidden");
    assert_eq!(check_metadata(&form).unwrap().0, "naming_pattern");
    form.field_mut("naming_pattern")
        .unwrap()
        .set_text("{date}_{id}");
    assert!(check_metadata(&form).is_none());
}

#[test]
fn an_id_width_outside_the_counters_range_is_a_typo_not_a_choice() {
    let mut form = id_form(&Template::default());
    form.field_mut("digits").unwrap().set_text("99");
    assert_eq!(check_id(&form).unwrap().0, "digits");
    form.field_mut("digits").unwrap().set_text("4");
    assert!(check_id(&form).is_none());
    form.field_mut("prefix").unwrap().set_text("  ");
    assert_eq!(check_id(&form).unwrap().0, "prefix");
}

#[test]
fn a_select_variable_needs_options_and_a_text_one_hides_them() {
    let mut form = variable_form(None);
    form.field_mut("slug").unwrap().set_text("tier");
    assert!(form.field("options").unwrap().hidden);
    form.field_mut("type").unwrap().select("select");
    sync_variable_form(&mut form, Glyphs::unicode());
    assert!(!form.field("options").unwrap().hidden);
    assert_eq!(variable_from(&form).unwrap_err().0, "options");
    form.field_mut("options")
        .unwrap()
        .set_text("Client, Internal");
    let variable = variable_from(&form).unwrap();
    assert_eq!(variable.options, vec!["Client", "Internal"]);
    assert_eq!(
        variable.label, "tier",
        "an empty label falls back to the slug"
    );
}

#[test]
fn a_reserved_filename_is_refused_where_it_is_typed() {
    let edit = FileEdit {
        index: 0,
        path: LineEdit::with_text("PROJECT_INFO.md"),
        body: TextArea::new(),
        in_body: false,
        error: None,
    };
    assert!(file_from(&edit).unwrap_err().contains("reserved"));
}

#[test]
fn an_empty_file_is_declarable() {
    let edit = FileEdit {
        index: 0,
        path: LineEdit::with_text(".gitkeep"),
        body: TextArea::new(),
        in_body: false,
        error: None,
    };
    let entry = file_from(&edit).unwrap();
    assert_eq!(entry.path, ".gitkeep");
    assert!(entry.template.is_empty(), "a marker file has no content");
}

#[test]
fn the_tokens_a_body_uses_are_the_ones_that_will_substitute() {
    let mut template = Template::default();
    let mut form = variable_form(None);
    form.field_mut("slug").unwrap().set_text("artist");
    template.variables.push(variable_from(&form).unwrap());
    assert_eq!(
        tokens_used("# {artist} — {date}\n{clientname}", &template),
        vec!["{artist}".to_string(), "{date}".to_string()]
    );
    assert!(tokens_used("plain text", &template).is_empty());
}

/// The mistake that costs a first template: declare `artist` and `title`,
/// leave the suggested `{date}_{id}` alone, and every project gets the
/// same folder name. It loads, it saves, and nothing says so until the
/// first create.
#[test]
fn a_pattern_that_ignores_its_variables_is_named_as_the_problem_it_is() {
    let mut template = Template {
        naming_pattern: "{date}_{id}".to_string(),
        ..Template::default()
    };
    assert_eq!(
        pattern_warning(&template),
        None,
        "with no variables declared there is nothing to leave out"
    );

    for slug in ["artist", "title"] {
        let mut form = variable_form(None);
        form.field_mut("slug").unwrap().set_text(slug);
        template.variables.push(variable_from(&form).unwrap());
    }
    let warning = pattern_warning(&template).expect("both variables are unused");
    assert!(warning.contains("{artist}"), "{warning}");
    assert!(warning.contains("{title}"), "{warning}");
    assert!(
        warning.contains("same folder name"),
        "it says what goes wrong, not just what is missing: {warning}"
    );

    template.naming_pattern = "{date}_{artist}_{title}_{id}".to_string();
    assert_eq!(pattern_warning(&template), None, "now every one is used");
}

/// The other half: a token no variable answers stays in the folder name
/// exactly as typed, braces and all. `{clientname}` for `client_name`.
#[test]
fn a_token_no_variable_answers_is_named_before_it_reaches_a_folder() {
    let mut template = Template::default();
    let mut form = variable_form(None);
    form.field_mut("slug").unwrap().set_text("client_name");
    template.variables.push(variable_from(&form).unwrap());
    template.naming_pattern = "{clientname}_{id}".to_string();

    let warning = pattern_warning(&template).expect("the typo is not a variable");
    assert!(warning.contains("{clientname}"), "{warning}");
    assert!(warning.contains("matches no variable"), "{warning}");

    // The unknown token is reported ahead of the unused one: it is the
    // one that is simply wrong.
    template.naming_pattern = "{client_name}_{id}".to_string();
    assert_eq!(pattern_warning(&template), None);
}

#[test]
fn tokens_are_read_out_of_a_pattern_without_a_regex() {
    assert_eq!(tokens_in("{date}_{id}"), vec!["date", "id"]);
    assert_eq!(tokens_in("{a}{a}"), vec!["a"], "each one once");
    assert!(tokens_in("no tokens here").is_empty());
    assert!(tokens_in("{unclosed").is_empty(), "a brace is not a token");
    assert!(tokens_in("{}").is_empty(), "and neither is an empty pair");
}

/// An edit must not let the title rewrite the slug: the slug is the
/// template's directory name, and Save renames the directory to match it.
#[test]
fn an_existing_templates_slug_is_already_chosen() {
    let existing = Template {
        name: "Client project".to_string(),
        slug: "client-project".to_string(),
        ..Template::default()
    };

    let mut form = metadata_form(&existing);
    form.field_mut("name").unwrap().set_text("Something else");
    suggest_slug(&mut form);
    assert_eq!(
        form.value("slug"),
        "client-project",
        "a loaded slug was chosen the moment it was written to disk"
    );

    // A brand-new template still gets the suggestion.
    let mut fresh = metadata_form(&Template::default());
    fresh.field_mut("name").unwrap().set_text("My Music Video");
    suggest_slug(&mut fresh);
    assert_eq!(fresh.value("slug"), "my-music-video");
}

#[test]
fn the_transform_row_shows_what_it_would_do_to_an_answer() {
    let mut form = variable_form(None);
    form.field_mut("transform")
        .unwrap()
        .select("TitleUnderscore");
    sync_variable_form(&mut form, Glyphs::unicode());
    let hint = &form.field("transform").unwrap().hint;
    assert!(hint.contains("Ariana_Grande"), "{hint}");
    assert!(hint.contains('→'), "{hint}");

    sync_variable_form(&mut form, Glyphs::ascii());
    let hint = &form.field("transform").unwrap().hint;
    assert!(hint.contains("->") && !hint.contains('→'), "{hint}");
}

#[test]
fn the_slug_follows_the_name_until_one_is_typed() {
    let mut form = metadata_form(&Template::default());
    form.field_mut("name").unwrap().set_text("My Music Video");
    suggest_slug(&mut form);
    assert_eq!(form.value("slug"), "my-music-video");
    form.field_mut("name").unwrap().set_text("Something Else");
    suggest_slug(&mut form);
    assert_eq!(form.value("slug"), "something-else", "still following");

    let slug = form.field_mut("slug").unwrap();
    slug.touched = true;
    slug.set_text("chosen");
    form.field_mut("name").unwrap().set_text("Third Name");
    suggest_slug(&mut form);
    assert_eq!(form.value("slug"), "chosen", "a typed slug stays");
}
