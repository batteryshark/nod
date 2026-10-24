import XCTest
@testable import NodKit

final class NodAnswerCompositionTests: XCTestCase {
  func testRequiredTextRejectsEmptyAndWhitespace() {
    for text in ["", " ", "\n\t", "\u{2003}"] {
      XCTAssertFalse(NodAnswerComposition.canSend(text: text, isSubmitting: false))
    }
  }

  func testTextIsNotRewrittenAndSubmissionIsConstrained() {
    let text = "  Keep the full answer\nincluding Unicode 🐈  "
    XCTAssertTrue(NodAnswerComposition.canSend(text: text, isSubmitting: false))
    XCTAssertFalse(NodAnswerComposition.canSend(text: text, isSubmitting: true))
    XCTAssertEqual(text, "  Keep the full answer\nincluding Unicode 🐈  ")
  }
}
