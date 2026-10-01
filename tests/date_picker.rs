//! Mini date-picker widget tests (kittest): toggle opens the popup, day
//! click writes YYYY-MM-DD into the bound String and closes, month
//! stepping moves the viewed month.
#![allow(clippy::unwrap_used)]

use egui::accesskit::Role;
use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;

struct PickerHarness {
    value: String,
}

fn make_harness() -> Harness<'static, PickerHarness> {
    Harness::builder().with_size([500.0, 500.0]).build_ui_state(
        |ui, state: &mut PickerHarness| {
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut state.value).desired_width(110.0));
                adjutant::ui::date_picker::date_picker(ui, &mut state.value, "test-due");
            });
        },
        PickerHarness {
            value: String::new(),
        },
    )
}

fn field_value(h: &Harness<'static, PickerHarness>) -> Option<String> {
    h.query_all_by_role(Role::TextInput)
        .next()
        .and_then(|n| n.value())
}

#[test]
fn day_click_writes_date_and_closes() {
    let mut h = make_harness();
    h.run_steps(2);
    h.get_by_label_contains("Pick date (icon button)").click();
    h.run_steps(2);

    let this_month = chrono::Local::now().format("%B %Y").to_string();
    assert_eq!(h.query_all_by_label(&this_month).count(), 1, "popup open");

    h.get_by_label("15").click();
    h.run_steps(2);

    let value = field_value(&h).unwrap_or_default();
    assert!(
        value.ends_with("-15"),
        "day click writes the date, got {value:?}"
    );
    assert_eq!(
        h.query_all_by_label(&this_month).count(),
        0,
        "popup closed after selection"
    );
}

#[test]
fn month_stepping_moves_the_viewed_month() {
    let mut h = make_harness();
    h.run_steps(2);
    h.get_by_label_contains("Pick date (icon button)").click();
    h.run_steps(2);

    h.get_by_label_contains("Next month (icon button)").click();
    h.run_steps(2);
    let next_month = (chrono::Local::now() + chrono::Duration::days(32))
        .format("%B %Y")
        .to_string();
    assert_eq!(h.query_all_by_label(&next_month).count(), 1);

    h.get_by_label_contains("Previous month (icon button)")
        .click();
    h.run_steps(2);
    let this_month = chrono::Local::now().format("%B %Y").to_string();
    assert_eq!(h.query_all_by_label(&this_month).count(), 1);
}

#[test]
fn selected_day_seeds_the_viewed_month() {
    let mut h = Harness::builder().with_size([500.0, 500.0]).build_ui_state(
        |ui, state: &mut PickerHarness| {
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut state.value).desired_width(110.0));
                adjutant::ui::date_picker::date_picker(ui, &mut state.value, "seeded");
            });
        },
        PickerHarness {
            value: "2027-03-10".to_string(),
        },
    );
    h.run_steps(2);
    h.get_by_label_contains("Pick date (icon button)").click();
    h.run_steps(2);
    assert_eq!(
        h.query_all_by_label("March 2027").count(),
        1,
        "existing value seeds the popup's month"
    );
}
