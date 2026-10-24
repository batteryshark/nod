import XCTest

@MainActor final class NativeAnswerInteractionTests: XCTestCase {
  override func setUp() { super.setUp(); continueAfterFailure = false }
  private func capture(_ app: XCUIApplication, _ name: String) {
    let attachment = XCTAttachment(screenshot:app.screenshot())
    attachment.name = name; attachment.lifetime = .keepAlways
    add(attachment)
  }
  private func launch(_ args: [String] = []) -> XCUIApplication {
    let app = XCUIApplication(bundleIdentifier:"com.nod.isolated-native-fixture")
    app.launchArguments = args
    app.launch()
    return app
  }
  private func reveal(_ option: XCUIElement, in scroll: XCUIElement) {
    for _ in 0..<12 {
      if option.exists && option.isHittable && scroll.frame.contains(option.frame) { return }
      let viewport = scroll.frame
      let target = option.frame
      let distance = target.minY < viewport.minY ? min(viewport.height * 0.35, viewport.minY - target.minY + 20) : -min(viewport.height * 0.35, max(40, target.maxY - viewport.maxY + 20))
      let start = scroll.coordinate(withNormalizedOffset:CGVector(dx:0.5,dy:0.5))
      start.press(forDuration:0.05,thenDragTo:start.withOffset(CGVector(dx:0,dy:distance)),withVelocity:.slow,thenHoldForDuration:0.1)
    }
    XCTFail("Option never became fully visible: \(option.identifier)")
  }

  func testRealModalAllTwentyOptionsAndDestructiveCancelRestore() {
    let app = launch()
    app.buttons["answer-access"].tap()
    let scroll = app.scrollViews["answer-options"]
    XCTAssertTrue(scroll.waitForExistence(timeout:5))
    capture(app,"native-modal-first")
    var geometry: [String] = []
    for n in 1...20 {
      let button = app.buttons["answer-option-option-\(n)"]
      reveal(button, in:scroll)
      XCTAssertGreaterThanOrEqual(button.frame.height,44)
      XCTAssertTrue(scroll.frame.contains(button.frame))
      XCTAssertTrue(button.label.contains("Keep the entire synthetic maintenance record"))
      geometry.append("\(n): option=\(button.frame), viewport=\(scroll.frame), hittable=\(button.isHittable)")
    }
    let frames = XCTAttachment(string:geometry.joined(separator:"\n")); frames.name = "native-all-option-frames"; frames.lifetime = .keepAlways; add(frames)
    capture(app,"native-modal-last")
    app.buttons["answer-option-option-20"].tap()
    XCTAssertTrue(app.buttons["confirm-destructive-answer"].waitForExistence(timeout:3))
    app.buttons["cancel-destructive-answer"].tap()
    XCTAssertTrue(app.buttons["answer-option-option-20"].isHittable)
    app.buttons["answer-option-option-20"].tap()
    app.buttons["confirm-destructive-answer"].tap()
    XCTAssertTrue(app.staticTexts["Fixture activations 1"].waitForExistence(timeout:3))
    app.terminate()
  }

  func testRequiredTextActualKeyboardAndCancelDraft() {
    let app = launch()
    app.buttons["answer-access"].tap()
    let option = app.buttons["answer-option-option-17"]
    reveal(option,in:app.scrollViews["answer-options"])
    option.tap()
    let send = app.buttons["send-answer"]
    XCTAssertTrue(send.waitForExistence(timeout:3))
    XCTAssertFalse(send.isEnabled)
    let input = app.descendants(matching:.any).matching(identifier:"answer-text").firstMatch
    input.tap(); input.typeText("Synthetic retained notes")
    XCTAssertTrue(send.isEnabled)
    XCTAssertTrue(app.keyboards.firstMatch.exists)
    XCTAssertLessThanOrEqual(send.frame.maxY,app.keyboards.firstMatch.frame.minY)
    capture(app,"native-required-text-keyboard")
    app.buttons["Cancel"].tap()
    XCTAssertTrue(option.isHittable)
    option.tap()
    XCTAssertTrue(input.value.debugDescription.contains("Synthetic retained notes"))
    send.tap()
    XCTAssertTrue(app.staticTexts["Fixture activations 1"].waitForExistence(timeout:3))
    app.terminate()
  }

  func testFourDefaultControlsAndLargeTextLandscapeDisclosure() {
    for kind in ["choice","number","date","date_time"] {
      let app = launch(["--field",kind,"--large-text"])
      let control = app.descendants(matching:.any).matching(identifier:"answer-value").firstMatch
      XCTAssertTrue(control.waitForExistence(timeout:4))
      reveal(control,in:app.scrollViews["request-body"])
      XCTAssertTrue(app.staticTexts["Fixture activations 0"].exists)
      capture(app,"native-default-" + kind)
      XCUIDevice.shared.orientation = .landscapeLeft
      app.buttons["answer-access"].tap()
      let option = app.buttons["answer-option-option-1"]
      XCTAssertTrue(option.waitForExistence(timeout:4))
      XCTAssertTrue(option.isHittable)
      option.tap()
      XCTAssertTrue(app.staticTexts["Fixture activations 1"].waitForExistence(timeout:3))
      let expected = ["choice":"small","number":"4","date":"2028-02-29","date_time":"2026-10-25T09:30:00Z"][kind]!
      XCTAssertTrue(app.staticTexts["Fixture value " + expected].exists)
      app.terminate()
      XCUIDevice.shared.orientation = .portrait
    }
  }
}
