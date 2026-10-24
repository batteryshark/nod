import Foundation

public enum NodAnswerValue: Hashable, Sendable, Codable {
  case choice(String), number(String), date(String), dateTime(String)

  public var kind: NodAnswerField.Kind {
    switch self { case .choice: .choice; case .number: .number; case .date: .date; case .dateTime: .dateTime }
  }
  public var string: String {
    switch self { case .choice(let s), .number(let s), .date(let s), .dateTime(let s): s }
  }
  private enum Keys: String, CodingKey { case kind, choiceId = "choice_id", decimal, date, instant }
  public init(from decoder: Decoder) throws {
    let c = try decoder.container(keyedBy: Keys.self)
    let kind = try c.decode(NodAnswerField.Kind.self, forKey: .kind)
    let key: Keys = switch kind { case .choice: .choiceId; case .number: .decimal; case .date: .date; case .dateTime: .instant }
    try rejectUnknownAnswerKeys(decoder, allowed: ["kind", key.rawValue])
    let s = try c.decode(String.self, forKey: key)
    self = switch kind { case .choice: .choice(s); case .number: .number(s); case .date: .date(s); case .dateTime: .dateTime(s) }
  }
  public func encode(to encoder: Encoder) throws {
    var c = encoder.container(keyedBy: Keys.self)
    try c.encode(kind, forKey: .kind)
    let key: Keys = switch self { case .choice: .choiceId; case .number: .decimal; case .date: .date; case .dateTime: .instant }
    try c.encode(string, forKey: key)
  }
}

public struct NodAnswerChoice: Hashable, Sendable, Codable, Identifiable {
  public let id: String
  public let label: String
  public init(id: String, label: String) { self.id = id; self.label = label }
  private enum CodingKeys: String, CodingKey { case id, label }
  public init(from decoder: Decoder) throws {
    try rejectUnknownAnswerKeys(decoder, allowed: ["id", "label"])
    let c = try decoder.container(keyedBy: CodingKeys.self)
    id = try c.decode(String.self, forKey: .id); label = try c.decode(String.self, forKey: .label)
  }
}

public struct NodAnswerField: Hashable, Sendable, Codable {
  public enum Kind: String, Codable, Sendable { case choice, number, date; case dateTime = "date_time" }
  public let kind: Kind
  public let label: String
  public let choices: [NodAnswerChoice]?
  public let defaultValue: NodAnswerValue
  public init(kind: Kind, label: String, choices: [NodAnswerChoice]? = nil, defaultValue: NodAnswerValue) {
    self.kind = kind; self.label = label; self.choices = choices; self.defaultValue = defaultValue
  }
  private enum Keys: String, CodingKey { case kind, label, choices; case defaultValue = "default" }
  public func encode(to encoder: Encoder) throws {
    var c = encoder.container(keyedBy: Keys.self)
    try c.encode(kind, forKey: .kind); try c.encode(label, forKey: .label)
    try c.encodeIfPresent(choices, forKey: .choices); try c.encode(defaultValue, forKey: .defaultValue)
  }
  public init(from decoder: Decoder) throws {
    let c = try decoder.container(keyedBy: Keys.self)
    kind = try c.decode(Kind.self, forKey: .kind)
    try rejectUnknownAnswerKeys(decoder, allowed: kind == .choice ? ["kind", "label", "choices", "default"] : ["kind", "label", "default"])
    label = try c.decode(String.self, forKey: .label)
    choices = kind == .choice ? try c.decode([NodAnswerChoice].self, forKey: .choices) : nil
    defaultValue = try c.decode(NodAnswerValue.self, forKey: .defaultValue)
    guard isValidDefinition && isValid(defaultValue) else { throw DecodingError.dataCorruptedError(forKey: .defaultValue, in: c, debugDescription: "Invalid answer definition or default") }
  }
  private var isValidDefinition: Bool {
    func labelValid(_ s: String, _ max: Int) -> Bool {
      !s.isEmpty && s == s.trimmingCharacters(in: .whitespacesAndNewlines) && s.unicodeScalars.count <= max && !s.unicodeScalars.contains { $0.properties.generalCategory == .control }
    }
    guard labelValid(label, 64), (try? JSONEncoder().encode(self).count).map({ $0 <= 4096 }) == true else { return false }
    guard kind == .choice else { return choices == nil }
    guard let choices, (2...12).contains(choices.count), Set(choices.map(\.id)).count == choices.count, Set(choices.map(\.label)).count == choices.count else { return false }
    return choices.allSatisfy { labelValid($0.label, 48) && $0.id.range(of: #"\A[A-Za-z0-9_-]{1,40}\z"#, options: .regularExpression) != nil }
  }
  public func isValid(_ value: NodAnswerValue) -> Bool {
    guard value.kind == kind else { return false }
    switch value {
    case .choice(let id): return choices?.contains(where: { $0.id == id }) == true
    case .number(let s):
      return s != "-0" && s.range(of: #"\A-?(0|[1-9][0-9]{0,11})(\.[0-9]{0,5}[1-9])?\z"#, options: .regularExpression) != nil
    case .date(let s): return Self.calendarValue(s, kind: .date) != nil
    case .dateTime(let s): return Self.calendarValue(s, kind: .dateTime) != nil
    }
  }
  public static func formatter(_ kind: Kind) -> DateFormatter {
    let f = DateFormatter(); f.calendar = Calendar(identifier: .gregorian)
    f.locale = Locale(identifier: "en_US_POSIX"); f.timeZone = TimeZone(secondsFromGMT: 0)
    f.dateFormat = kind == .date ? "yyyy-MM-dd" : "yyyy-MM-dd'T'HH:mm':00Z'"
    f.isLenient = false
    f.gregorianStartDate = .distantPast
    return f
  }
  /// Foundation's native Gregorian picker retains the historical reform gap.
  /// Keep affected defaults exact in ONE ISO text control instead of displaying
  /// a shifted date. Validation remains the same proleptic Gregorian contract.
  public static func requiresISODateText(_ value: NodAnswerValue) -> Bool {
    (value.kind == .date || value.kind == .dateTime) && String(value.string.prefix(10)) < "1582-10-15"
  }
  public static var pickerCalendar: Calendar {
    var calendar = Calendar(identifier: .gregorian)
    calendar.timeZone = TimeZone(secondsFromGMT: 0)!
    return calendar
  }
  public static func pickerValue(_ date: Date, kind: Kind) -> NodAnswerValue {
    // Encode the civil components the picker actually displays. If a user
    // navigates into the historic range, the control switches to exact ISO text;
    // never reinterpret a Julian picker day through a proleptic formatter.
    let c = pickerCalendar.dateComponents([.era,.year,.month,.day,.hour,.minute], from:date)
    guard c.era == 1, let year = c.year, let month = c.month, let day = c.day else {
      return kind == .date ? .date("") : .dateTime("")
    }
    let dayString = String(format:"%04d-%02d-%02d",year,month,day)
    if kind == .date { return .date(dayString) }
    return .dateTime(dayString + String(format:"T%02d:%02d:00Z",c.hour ?? 0,c.minute ?? 0))
  }
  public static func calendarValue(_ string: String, kind: Kind) -> Date? {
    guard !string.hasPrefix("0000"), string.utf8.count == (kind == .date ? 10 : 20) else { return nil }
    let f = formatter(kind)
    guard let date = f.date(from: string), f.string(from: date) == string else { return nil }
    return date
  }
}

private struct AnswerKey: CodingKey {
  let stringValue: String
  var intValue: Int? { nil }
  init?(stringValue: String) { self.stringValue = stringValue }
  init?(intValue: Int) { return nil }
}
private func rejectUnknownAnswerKeys(_ decoder: Decoder, allowed: Set<String>) throws {
  let c = try decoder.container(keyedBy: AnswerKey.self)
  guard c.allKeys.allSatisfy({ allowed.contains($0.stringValue) }) else {
    throw DecodingError.dataCorrupted(.init(codingPath: decoder.codingPath, debugDescription: "Unsupported answer field key"))
  }
}
