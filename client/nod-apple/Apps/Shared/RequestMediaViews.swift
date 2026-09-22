import NodKit
import SwiftUI
import ImageIO

#if os(iOS) || os(macOS)
@preconcurrency import LinkPresentation
#endif

func resolvedImageURL(_ value: String?) -> URL? {
  guard let value else {
    return nil
  }
  return normalizedWebURL(from: value)
}

func resolvedLink(_ link: NodLink) -> (label: String, url: URL)? {
  let rawURL = link.url.trimmingCharacters(in: .whitespacesAndNewlines)
  guard let url = normalizedWebURL(from: rawURL) else {
    return nil
  }

  let rawLabel = link.label.trimmingCharacters(in: .whitespacesAndNewlines)
  let label = rawLabel == "https" && rawURL.hasPrefix("//") ? linkLabel(from: url) : rawLabel
  return (label.isEmpty ? linkLabel(from: url) : label, url)
}

private func linkLabel(from url: URL) -> String {
  let path = url.path == "/" ? "" : url.path
  if let host = url.host, !host.isEmpty {
    return host + path
  }
  return url.absoluteString
}

private func normalizedWebURL(from value: String) -> URL? {
  let rawURL = value.trimmingCharacters(in: .whitespacesAndNewlines)
  let normalizedURL: String
  if rawURL.hasPrefix("//") {
    normalizedURL = "https:" + rawURL
  } else if rawURL.lowercased().hasPrefix("www.") {
    normalizedURL = "https://" + rawURL
  } else {
    normalizedURL = rawURL
  }

  guard let url = URL(string: normalizedURL),
    let scheme = url.scheme?.lowercased(),
    ["http", "https"].contains(scheme)
  else {
    return nil
  }
  return url
}

struct RequestImageView: View {
  let url: URL
  @AppStorage("nod.loadRemoteMedia") private var loadRemoteMedia = false
  @State private var requested = false
  @State private var image: CGImage?
  @State private var failed = false

  var body: some View {
    Group {
      if let image {
        Image(decorative: image, scale: 1)
          .resizable().scaledToFit().frame(maxWidth: .infinity, maxHeight: 420)
          .clipShape(RoundedRectangle(cornerRadius: 8))
      } else if failed {
        Link(destination: url) { Label("Open Image", systemImage: "photo") }
      } else if loadRemoteMedia || requested {
        ProgressView("Loading image…").frame(maxWidth: .infinity, minHeight: 80)
      } else {
        Button { requested = true } label: { Label("Load image from " + (url.host ?? "remote site"), systemImage: "photo") }
      }
    }
    .task(id: "\(url.absoluteString):\(loadRemoteMedia || requested)") {
      guard loadRemoteMedia || requested else { return }
      image = nil
      failed = false
      do {
        let data = try await boundedImageData(url)
        guard let source = CGImageSourceCreateWithData(data as CFData, nil),
              let properties = CGImageSourceCopyPropertiesAtIndex(source, 0, nil) as? [CFString: Any],
              let width = properties[kCGImagePropertyPixelWidth] as? Int,
              let height = properties[kCGImagePropertyPixelHeight] as? Int,
              width > 0, height > 0, width <= 4096, height <= 4096,
              let thumbnail = CGImageSourceCreateThumbnailAtIndex(source, 0, [
                kCGImageSourceCreateThumbnailFromImageAlways: true,
                kCGImageSourceThumbnailMaxPixelSize: 1600,
                kCGImageSourceCreateThumbnailWithTransform: true
              ] as CFDictionary) else { throw URLError(.cannotDecodeContentData) }
        image = thumbnail
      } catch is CancellationError { } catch { failed = true }
    }
  }

  nonisolated private func boundedImageData(_ url: URL) async throws -> Data {
    let config = URLSessionConfiguration.ephemeral
    config.timeoutIntervalForRequest = 8
    config.timeoutIntervalForResource = 12
    let session = URLSession(configuration: config)
    defer { session.invalidateAndCancel() }
    let (bytes, response) = try await session.bytes(from: url)
    let maximumBytes = 8 * 1024 * 1024
    guard let response = response as? HTTPURLResponse, (200..<300).contains(response.statusCode),
          response.expectedContentLength <= maximumBytes,
          response.mimeType?.hasPrefix("image/") == true else { throw URLError(.badServerResponse) }
    var data = Data()
    for try await byte in bytes {
      guard data.count < maximumBytes else { throw URLError(.dataLengthExceedsMaximum) }
      data.append(byte)
    }
    return data
  }
}

struct RequestLinkView: View {
  let label: String
  let url: URL

  var body: some View {
    #if os(iOS) || os(macOS)
      LinkPreviewCard(label: label, url: url)
    #else
      Link(destination: url) {
        Label(label, systemImage: "arrow.up.right.square")
      }
    #endif
  }
}

#if os(iOS) || os(macOS)
private struct LinkPreviewCard: View {
  @Environment(\.openURL) private var openURL
  @StateObject private var model = LinkPreviewModel()
  @AppStorage("nod.loadRemoteMedia") private var loadRemoteMedia = false
  let label: String
  let url: URL

  var body: some View {
    Button {
      openURL(url)
    } label: {
      Group {
        if let metadata = model.metadata {
          LinkPreviewRepresentable(metadata: metadata)
            .frame(height: 96)
            .clipShape(RoundedRectangle(cornerRadius: 8))
        } else {
          HStack(spacing: 10) {
            Image(systemName: "link")
              .font(.headline)
              .frame(width: 28, height: 28)
              .foregroundStyle(.tint)
            VStack(alignment: .leading, spacing: 2) {
              Text(label)
                .font(.subheadline.weight(.semibold))
                .foregroundStyle(.primary)
                .lineLimit(2)
              Text(url.host ?? url.absoluteString)
                .font(.caption)
                .foregroundStyle(.secondary)
                .lineLimit(1)
            }
            Spacer(minLength: 8)
            Image(systemName: "arrow.up.right.square")
              .foregroundStyle(.secondary)
          }
          .padding(12)
          .background(.quaternary, in: RoundedRectangle(cornerRadius: 8))
        }
      }
      .frame(maxWidth: .infinity, alignment: .leading)
    }
    .buttonStyle(.plain)
    .task(id: "\(url.absoluteString):\(loadRemoteMedia)") {
      if loadRemoteMedia { model.load(url: url) } else { model.cancel() }
    }
    .onDisappear { model.cancel() }
  }
}

@MainActor
private final class LinkPreviewModel: ObservableObject {
  @Published var metadata: LPLinkMetadata?
  private var requestedURL: URL?
  private var provider: LPMetadataProvider?

  func cancel() {
    provider?.cancel()
    provider = nil
    requestedURL = nil
    metadata = nil
  }

  func load(url: URL) {
    guard requestedURL != url else {
      return
    }
    provider?.cancel()
    requestedURL = url
    metadata = nil

    let provider = LPMetadataProvider()
    provider.timeout = 8
    provider.shouldFetchSubresources = false
    self.provider = provider
    provider.startFetchingMetadata(for: url) { [weak self] metadata, _ in
      let decision = LinkMetadataResult(metadata: metadata)
      Task { @MainActor [weak self, decision] in
        guard let self, self.requestedURL == url else {
          return
        }
        self.metadata = decision.metadata
        self.provider = nil
      }
    }
  }
}

private struct LinkMetadataResult: @unchecked Sendable {
  let metadata: LPLinkMetadata?
}

#if os(iOS)
private struct LinkPreviewRepresentable: UIViewRepresentable {
  let metadata: LPLinkMetadata

  func makeUIView(context: Context) -> LPLinkView {
    LPLinkView(metadata: metadata)
  }

  func updateUIView(_ uiView: LPLinkView, context: Context) {
    uiView.metadata = metadata
  }
}
#elseif os(macOS)
private struct LinkPreviewRepresentable: NSViewRepresentable {
  let metadata: LPLinkMetadata

  func makeNSView(context: Context) -> LPLinkView {
    LPLinkView(metadata: metadata)
  }

  func updateNSView(_ nsView: LPLinkView, context: Context) {
    nsView.metadata = metadata
  }
}
#endif
#endif
