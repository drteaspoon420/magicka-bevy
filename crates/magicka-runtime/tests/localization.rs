use magicka_runtime::{
    dialog_definition::DialogDefinition,
    localization::{LocalizationError, LocalizationTable},
    scene_loading::custom_hash,
};
fn text(
    key: &str,
    arguments: Option<Vec<i32>>,
) -> magicka_runtime::dialog_definition::TextDefinition {
    magicka_runtime::dialog_definition::TextDefinition {
        ttl: -1.0,
        animation: 151,
        sound_bank: 32,
        sound: 0,
        localization: custom_hash(key),
        arguments,
    }
}
fn table(rows: &str) -> LocalizationTable {
    let mut table = LocalizationTable::default();
    table
        .add_xml(
            format!("<Workbook><Worksheet><Table>{rows}</Table></Worksheet></Workbook>").as_bytes(),
        )
        .unwrap();
    table
}
#[test]
fn workbook_inner_text_overwrites_and_untrimmed_keys() {
    let mut table = table(
        r##"
        <Row><Cell>header</Cell><Cell>ignored</Cell></Row>
        <Row><Cell>
            <Data>#KeY</Data>
        </Cell><Cell><Data>ä &amp; b\nnext</Data></Cell></Row>
        <Row><Cell>#key</Cell><Cell/></Row>
        <Row><Cell> #key</Cell><Cell>ignored</Cell></Row>
        <Row><Cell>#trail </Cell><Cell>kept</Cell></Row>
        <Row><Cell>only</Cell></Row>
    "##,
    );
    assert_eq!(table.strings.len(), 2);
    assert_eq!(
        table.get_string(custom_hash("#key")).unwrap(),
        "ä & b\nnext"
    );
    assert_eq!(table.get_string(custom_hash("#trail ")).unwrap(), "kept");
    assert_eq!(table.add_xml(b"<workbook><worksheet><table><row><a>#KEY</a><b>new</b></row></table></worksheet></workbook>"), Ok(1));
    assert_eq!(table.get_string(custom_hash("#key")).unwrap(), "new");
    assert_eq!(table.get_string(-123).unwrap(), "NOT FOUND: -123");
}
#[test]
fn xml_space_cdata_and_explicit_row_limits() {
    let table = table("<Row><a>#space</a><b xml:space=\"preserve\"> <x xml:space=\"default\"> <y>A</y> </x> </b></Row><Row><a>#cdata</a><b><![CDATA[ ]]></b></Row>");
    assert_eq!(table.get_string(custom_hash("#space")).unwrap(), " A ");
    assert_eq!(table.get_string(custom_hash("#cdata")).unwrap(), " ");
    let mut table = LocalizationTable::default();
    assert_eq!(table.add_xml(b"<Workbook><Worksheet><Table><Row><!--x--><a>#a</a><b>b</b></Row></Table></Worksheet></Workbook>"), Err(LocalizationError::UnsupportedRowNodes));
    assert_eq!(
        table.add_xml(b"<ss:Workbook xmlns:ss=\"urn:x\"/>"),
        Err(LocalizationError::MissingWorkbook)
    );
}
#[test]
fn players_cache_nullable_stale_and_no_newline_conversion() {
    let mut table = LocalizationTable::default();
    let id = custom_hash("#players");
    assert_eq!(table.get_string(id), None);
    assert_eq!(
        table.expand_message(&text("#players", None), 10),
        Err(LocalizationError::NullString(id))
    );
    table.set_player_names(&["A\\nB", "C", "D"]);
    assert_eq!(table.get_string(id).unwrap(), "A\\nB, C & D");
    table.set_player_names(&[]);
    assert_eq!(table.get_string(id).unwrap(), "A\\nB, C & D");
    table.set_player_names(&["A", "B"]);
    assert_eq!(table.get_string(id).unwrap(), "A & B");
    table.set_player_names(&["A"]);
    assert_eq!(table.get_string(id).unwrap(), "A");
}
#[test]
fn loaded_dialog_numeric_named_and_nested_references() {
    let table = table("<Row><a>#main</a><b>Hello #1; #NAme; #players; end #literal</b></Row><Row><a>#arg</a><b>#name;</b></Row><Row><a>#name</a><b>Wizard</b></Row>");
    let defs = DialogDefinition::read_collection(&[b"<Dialogs><Dialog><Interact><Message><Text>\n  <Key>#main,#arg</Key>\n</Text></Message></Interact></Dialog></Dialogs>"]).unwrap();
    let value = table
        .expand_message(&defs[0].interactions[0].messages[0].texts[0], 10)
        .unwrap();
    assert_eq!(
        value,
        "Hello [c=1,1,1][c=1,1,1]Wizard[/c][/c] [c=1,1,1]Wizard[/c] [c=1,1,1][/c] end #literal"
    );
}
#[test]
fn whole_string_rescan_and_argument_failures() {
    let mut table = LocalizationTable::default();
    table
        .strings
        .insert(custom_hash("#main"), "#first; #last;".into());
    table
        .strings
        .insert(custom_hash("#first"), "#unterminated".into());
    let token = "#unterminated[/c] #last";
    assert_eq!(
        table.expand_message(&text("#main", None), 10).unwrap(),
        format!("[c=1,1,1][c=1,1,1]NOT FOUND: {}[/c]", custom_hash(token))
    );
    for number in ["0", "-1", "2", "-2147483648"] {
        table.strings.insert(1, format!("#{number};"));
        let mut def = text("#main", Some(vec![3]));
        def.localization = 1;
        assert_eq!(
            table.expand_message(&def, 10),
            Err(LocalizationError::ArgumentIndex(number.parse().unwrap()))
        );
        def.arguments = None;
        assert_eq!(
            table.expand_message(&def, 10),
            Err(LocalizationError::MissingArguments)
        );
    }
    table.strings.insert(1, "#+1;".into());
    table.strings.insert(3, "works".into());
    let mut def = text("#main", Some(vec![3]));
    def.localization = 1;
    assert_eq!(table.expand_message(&def, 1).unwrap(), "[c=1,1,1]works[/c]");
}
#[test]
fn cycles_fail_explicitly_at_caller_budget() {
    let table = table("<Row><a>#cycle</a><b>#cycle;</b></Row>");
    assert_eq!(
        table.expand_message(&text("#cycle", None), 3),
        Err(LocalizationError::SubstitutionLimit)
    );
}

#[test]
fn entity_text_runs_keep_spaces_and_nbsp_identifiers() {
    let table =
        table("<Row><a>#entity</a><b> &amp; </b></Row><Row><a>#NBSP&#160;</a><b>kept</b></Row>");
    assert_eq!(table.get_string(custom_hash("#entity")).unwrap(), " & ");
    assert_eq!(
        table.get_string(custom_hash("#nbsp\u{a0}")).unwrap(),
        "kept"
    );
}
