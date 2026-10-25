import XCTest
@testable import NodKit

final class NodAnswerValueTests: XCTestCase {
  func testProlepticGregorianHistoricDatesAndUTCMinutes() throws {
    for kind in [NodAnswerField.Kind.date, .dateTime] {
      let field = NodAnswerField(kind:kind,label:"Day",defaultValue:kind == .date ? .date("1600-02-29") : .dateTime("1600-02-29T09:30:00Z"))
      for day in ["1500-02-29","1600-02-29","1582-10-10","0001-01-01","9999-12-31"] {
        let text = kind == .date ? day : day + "T09:30:00Z"
        let value: NodAnswerValue = kind == .date ? .date(text) : .dateTime(text)
        if day == "1500-02-29" {
          XCTAssertFalse(field.isValid(value),text)
          XCTAssertNil(NodAnswerField.calendarValue(text,kind:kind))
        } else {
          XCTAssertTrue(field.isValid(value),text)
          let parsed = try XCTUnwrap(NodAnswerField.calendarValue(text,kind:kind))
          XCTAssertEqual(NodAnswerField.formatter(kind).string(from:parsed),text)
          XCTAssertEqual(NodAnswerField.requiresISODateText(value),day < "1582-10-15")
        }
      }
    }
    // A historic valid default never enters the cutover-based native picker.
    let exact = NodAnswerValue.date("1582-10-10")
    XCTAssertTrue(NodAnswerField.requiresISODateText(exact))
    XCTAssertEqual(exact.string,"1582-10-10")
    let modern = try XCTUnwrap(NodAnswerField.calendarValue("2026-10-25T09:30:00Z",kind:.dateTime))
    XCTAssertEqual(NodAnswerField.pickerValue(modern,kind:.dateTime),.dateTime("2026-10-25T09:30:00Z"))
  }
  func testValueReceiptDecodesActualSignatureVersionWithoutInventingVerification() throws {
    let raw = #"{"request_id":"r","option_id":"yes","option_kind":"custom","option_label":"Yes","answer_value":{"kind":"number","decimal":"4"},"resolved_at":"2026-10-24T00:00:00Z","signature":{"payload_version":"nod-decision-v2","key_id":"synthetic","algorithm":"p256","nonce":"synthetic","signed_at":"2026-10-24T00:00:00Z","request_digest":"synthetic","signing_payload":"synthetic","signature":"synthetic","verified":false}}"#
    let decision = try JSONDecoder.nod.decode(NodDecision.self,from:Data(raw.utf8))
    XCTAssertEqual(decision.answerValue,.number("4"))
    XCTAssertEqual(decision.signature?.payloadVersion,"nod-decision-v2")
    XCTAssertEqual(decision.signature?.verified,false)
  }
  func testOrdinaryFieldsOmittedAndValuePlaceholderNeverMeansInformation() throws {
    let raw = #"{"id":"r","request_id":"r","channel_id":"c","title":"Ordinary","summary":"","body_markdown":"","notification":{"redact":false},"status":"pending","created_at":"2026-10-24T00:00:00Z","updated_at":"2026-10-24T00:00:00Z","options":[]}"#
    let ordinary = try JSONDecoder.nod.decode(NodRequest.self, from:Data(raw.utf8))
    let encoded = try XCTUnwrap(JSONSerialization.jsonObject(with:JSONEncoder().encode(ordinary)) as? [String:Any])
    XCTAssertNil(encoded["answer_field"])
    XCTAssertFalse(ordinary.isValueRequest)
    var placeholder = try XCTUnwrap(JSONSerialization.jsonObject(with:Data(raw.utf8)) as? [String:Any])
    // A bare nod-request-v2 context without content is the legacy ordinary tag,
    // not a value card.
    placeholder["signing"] = ["version":"nod-request-v2","recipients_commitment":"synthetic","request_digest":"synthetic"]
    let legacyOrdinary = try JSONDecoder.nod.decode(NodRequest.self,from:JSONSerialization.data(withJSONObject:placeholder))
    XCTAssertFalse(legacyOrdinary.isValueRequest)
    // A card already issued under the legacy value tag stays value-only.
    placeholder["signing"] = ["version":"nod-request-v3","recipients_commitment":"synthetic","request_digest":"synthetic"]
    let legacyUnsupported = try JSONDecoder.nod.decode(NodRequest.self,from:JSONSerialization.data(withJSONObject:placeholder))
    XCTAssertTrue(legacyUnsupported.isValueRequest)
    placeholder["answer_field"] = NSNull()
    XCTAssertThrowsError(try JSONDecoder.nod.decode(NodRequest.self,from:JSONSerialization.data(withJSONObject:placeholder)))
    let decision = NodDecision(requestId:"r",optionId:"yes",optionKind:.custom,optionLabel:"Yes",resolvedAt:Date(timeIntervalSince1970:0))
    let decisionJSON = try XCTUnwrap(JSONSerialization.jsonObject(with:JSONEncoder().encode(decision)) as? [String:Any])
    XCTAssertNil(decisionJSON["answer_value"])
  }
  func testExactFourTaggedValuesAndDefaultsRoundTrip() throws {
    for raw in [
      #"{"kind":"choice","label":"Batch","choices":[{"id":"small","label":"Small"},{"id":"large","label":"Large"}],"default":{"kind":"choice","choice_id":"small"}}"#,
      #"{"kind":"number","label":"Amount","default":{"kind":"number","decimal":"999999999999.123456"}}"#,
      #"{"kind":"date","label":"Day","default":{"kind":"date","date":"2028-02-29"}}"#,
      #"{"kind":"date_time","label":"Time","default":{"kind":"date_time","instant":"2026-10-25T09:30:00Z"}}"#
    ] {
      let original = Data(raw.utf8)
      let field = try JSONDecoder().decode(NodAnswerField.self, from: original)
      XCTAssertTrue(field.isValid(field.defaultValue))
      let encoded = try JSONEncoder().encode(field)
      XCTAssertEqual(try JSONSerialization.jsonObject(with: original) as? NSDictionary, try JSONSerialization.jsonObject(with: encoded) as? NSDictionary)
    }
  }

  func testExactDecimalAndCalendarConstraintsWithoutDouble() {
    let number = NodAnswerField(kind:.number, label:"Number", defaultValue:.number("4"))
    for s in ["4", "1.5", "-0.25", "999999999999.123456", "0"] { XCTAssertTrue(number.isValid(.number(s)), s) }
    for s in ["1.50", "-0", "-0.0", "01", "+1", "1e2", " 4", "4\n", "NaN", "1000000000000", "0.1234567"] { XCTAssertFalse(number.isValid(.number(s)), s) }
    let date = NodAnswerField(kind:.date, label:"Date", defaultValue:.date("2028-02-29"))
    XCTAssertFalse(date.isValid(.date("2027-02-29")))
    XCTAssertFalse(date.isValid(.date("0000-01-01")))
    let time = NodAnswerField(kind:.dateTime, label:"Time", defaultValue:.dateTime("2026-10-25T09:30:00Z"))
    XCTAssertFalse(time.isValid(.dateTime("2026-10-25T09:30:01Z")))
    XCTAssertFalse(time.isValid(.dateTime("2026-10-25T24:30:00Z")))
    XCTAssertFalse(time.isValid(.number("4")))
  }

  func testRejectsUnknownFieldsNullDefaultsAndWrongTypes() {
    for raw in [
      #"{"kind":"number","label":"Count","scale":0,"default":{"kind":"number","decimal":"4"}}"#,
      #"{"kind":"number","label":"Count","default":null}"#,
      #"{"kind":"number","label":"Count","default":{"kind":"number","decimal":4}}"#,
      #"{"kind":"number","label":"Count","default":{"kind":"date","date":"2026-10-25"}}"#,
      #"{"kind":"choice","label":"Pick","choices":[],"default":{"kind":"choice","choice_id":"x"}}"#
    ] { XCTAssertThrowsError(try JSONDecoder().decode(NodAnswerField.self, from: Data(raw.utf8)), raw) }
  }
}
