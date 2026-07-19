import Foundation
#if canImport(FoundationModels) && !BATCAVE_FOUNDATION_MODELS_UNAVAILABLE
import FoundationModels
#endif

let sidecarProtocolVersion = 1
let maximumInputBytes = 32 * 1024
let maximumOutputBytes = 4 * 1024
let maximumNarrativeCharacters = 180

enum ProviderAvailability: String, Codable {
    case available
    case unsupported
    case modelNotReady = "model_not_ready"
    case runtimeMissing = "runtime_missing"
    case busy
}

enum SidecarOperation: String, Decodable {
    case status
    case generate
}

struct SidecarRequest: Decodable {
    let version: Int
    let operation: SidecarOperation
    let request: GenerationRequest?
    let facts: JSONValue?
}

struct GenerationRequest: Decodable {
    let surface: String
    let publicationSequence: UInt64
    let factDigest: String

    enum CodingKeys: String, CodingKey {
        case surface
        case publicationSequence = "publication_seq"
        case factDigest = "fact_digest"
    }
}

struct SidecarResponse: Encodable {
    let version = sidecarProtocolVersion
    let availability: ProviderAvailability
    let result: GenerationResult?
}

struct GenerationResult: Encodable {
    let provider = "apple_foundation"
    let publicationSequence: UInt64
    let factDigest: String
    let text: String

    enum CodingKeys: String, CodingKey {
        case provider
        case publicationSequence = "publication_seq"
        case factDigest = "fact_digest"
        case text
    }
}

indirect enum JSONValue: Codable {
    case null
    case bool(Bool)
    case number(Decimal)
    case string(String)
    case array([JSONValue])
    case object([String: JSONValue])

    init(from decoder: Decoder) throws {
        let container = try decoder.singleValueContainer()
        if container.decodeNil() {
            self = .null
        } else if let value = try? container.decode(Bool.self) {
            self = .bool(value)
        } else if let value = try? container.decode(Decimal.self) {
            self = .number(value)
        } else if let value = try? container.decode(String.self) {
            self = .string(value)
        } else if let value = try? container.decode([JSONValue].self) {
            self = .array(value)
        } else if let value = try? container.decode([String: JSONValue].self) {
            self = .object(value)
        } else {
            throw DecodingError.dataCorruptedError(
                in: container,
                debugDescription: "unsupported JSON value"
            )
        }
    }

    func encode(to encoder: Encoder) throws {
        var container = encoder.singleValueContainer()
        switch self {
        case .null:
            try container.encodeNil()
        case .bool(let value):
            try container.encode(value)
        case .number(let value):
            try container.encode(value)
        case .string(let value):
            try container.encode(value)
        case .array(let value):
            try container.encode(value)
        case .object(let value):
            try container.encode(value)
        }
    }
}

enum SidecarProtocolError: Error {
    case invalidRequest
    case requestTooLarge
    case responseTooLarge
}

func decodeRequest(_ data: Data) throws -> SidecarRequest {
    guard !data.isEmpty, data.count <= maximumInputBytes else {
        throw data.isEmpty ? SidecarProtocolError.invalidRequest : SidecarProtocolError.requestTooLarge
    }
    let request = try JSONDecoder().decode(SidecarRequest.self, from: data)
    guard request.version == sidecarProtocolVersion else {
        throw SidecarProtocolError.invalidRequest
    }
    switch request.operation {
    case .status:
        guard request.request == nil, request.facts == nil else {
            throw SidecarProtocolError.invalidRequest
        }
    case .generate:
        guard let generation = request.request,
              request.facts != nil,
              !generation.surface.isEmpty,
              generation.surface.count <= 32,
              !generation.factDigest.isEmpty,
              generation.factDigest.count <= 128
        else {
            throw SidecarProtocolError.invalidRequest
        }
    }
    return request
}

func encodeResponse(_ response: SidecarResponse) throws -> Data {
    let encoder = JSONEncoder()
    encoder.outputFormatting = [.sortedKeys, .withoutEscapingSlashes]
    var data = try encoder.encode(response)
    guard data.count + 1 <= maximumOutputBytes else {
        throw SidecarProtocolError.responseTooLarge
    }
    data.append(0x0a)
    return data
}

func canonicalFacts(_ facts: JSONValue) throws -> String {
    let encoder = JSONEncoder()
    encoder.outputFormatting = [.sortedKeys, .withoutEscapingSlashes]
    let data = try encoder.encode(facts)
    guard let value = String(data: data, encoding: .utf8) else {
        throw SidecarProtocolError.invalidRequest
    }
    return value
}

func groundingValues(_ facts: JSONValue) throws -> (displayName: String, resource: String) {
    guard case .object(let object) = facts,
          case .string(let displayName) = object["display_name"],
          case .string(let leadingResource) = object["leading_resource"]
    else {
        throw SidecarProtocolError.invalidRequest
    }
    let resource: String
    switch leadingResource {
    case "cpu": resource = "CPU"
    case "memory": resource = "memory"
    case "io": resource = "disk"
    case "network": resource = "network"
    default: throw SidecarProtocolError.invalidRequest
    }
    return (displayName, resource)
}

func normalizeOneSentence(_ generated: String) -> String? {
    let collapsed = generated
        .split(whereSeparator: { $0.isWhitespace })
        .joined(separator: " ")
        .trimmingCharacters(in: .whitespacesAndNewlines)
    guard !collapsed.isEmpty else { return nil }

    let sentence: String
    if let expression = try? NSRegularExpression(pattern: #"[.!?](?:[\"']?)(?:\s|$)"#),
       let match = expression.firstMatch(
           in: collapsed,
           range: NSRange(collapsed.startIndex..., in: collapsed)
       ),
       let range = Range(match.range, in: collapsed)
    {
        sentence = String(collapsed[..<range.upperBound]).trimmingCharacters(in: .whitespaces)
    } else {
        sentence = collapsed
    }

    let terminalCharacters = CharacterSet(charactersIn: ".!?")
    let needsTerminal = sentence.unicodeScalars.last.map { !terminalCharacters.contains($0) } ?? true
    let contentLimit = maximumNarrativeCharacters - (needsTerminal ? 1 : 0)
    var bounded = sentence.count > contentLimit ? String(sentence.prefix(contentLimit)) : sentence
    bounded = bounded.trimmingCharacters(in: .whitespacesAndNewlines)
    if needsTerminal {
        bounded = bounded.trimmingCharacters(in: CharacterSet(charactersIn: ",;:-")) + "."
    }
    if bounded.count > maximumNarrativeCharacters {
        bounded = String(bounded.prefix(maximumNarrativeCharacters - 1)) + "."
    }
    return bounded.isEmpty ? nil : bounded
}

func currentModelAvailability() -> ProviderAvailability {
#if canImport(FoundationModels) && !BATCAVE_FOUNDATION_MODELS_UNAVAILABLE
    guard #available(macOS 26.0, *) else { return .unsupported }
    return providerAvailability(SystemLanguageModel.default.availability)
#else
    return .unsupported
#endif
}

#if canImport(FoundationModels) && !BATCAVE_FOUNDATION_MODELS_UNAVAILABLE
@available(macOS 26.0, *)
func providerAvailability(
    _ availability: SystemLanguageModel.Availability
) -> ProviderAvailability {
    switch availability {
    case .available:
        return .available
    case .unavailable(let reason):
        switch reason {
        case .modelNotReady:
            return .modelNotReady
        case .deviceNotEligible, .appleIntelligenceNotEnabled:
            return .unsupported
        @unknown default:
            return .runtimeMissing
        }
    @unknown default:
        return .runtimeMissing
    }
}
#endif

func handleRequest(_ request: SidecarRequest) async -> SidecarResponse {
    let availability = currentModelAvailability()
    guard request.operation == .generate else {
        return SidecarResponse(availability: availability, result: nil)
    }
    guard availability == .available,
          let generation = request.request,
          let facts = request.facts
    else {
        return SidecarResponse(availability: availability, result: nil)
    }
#if canImport(FoundationModels) && !BATCAVE_FOUNDATION_MODELS_UNAVAILABLE
    guard #available(macOS 26.0, *) else {
        return SidecarResponse(availability: .unsupported, result: nil)
    }
    return await generate(generation, facts: facts)
#else
    return SidecarResponse(availability: .unsupported, result: nil)
#endif
}

#if canImport(FoundationModels) && !BATCAVE_FOUNDATION_MODELS_UNAVAILABLE
@available(macOS 26.0, *)
private func generate(_ request: GenerationRequest, facts: JSONValue) async -> SidecarResponse {
    do {
        let factsJSON = try canonicalFacts(facts)
        let grounding = try groundingValues(facts)
        let preferred = request.surface == "overview_contributor"
            ? "\(grounding.displayName) is the leading \(grounding.resource) contributor right now."
            : "\(grounding.displayName) is showing notable \(grounding.resource) activity right now."
        let model = SystemLanguageModel.default
        guard providerAvailability(model.availability) == .available else {
            return SidecarResponse(
                availability: providerAvailability(model.availability),
                result: nil
            )
        }
        let session = LanguageModelSession(
            model: model,
            instructions: "Write one short monitoring sentence using only the supplied trusted facts. The sentence must include the exact supplied display_name and leading_resource words. Never state a metric number; preserve any number that is part of the exact display_name. Never infer a cause, recommendation, path, process ID, or identity that is absent from those facts."
        )
        let prompt = """
            Monitoring surface: \(request.surface)
            Trusted fact packet JSON: \(factsJSON)
            Preferred sentence shape: \(preferred)
            Return that sentence or a shorter equivalent using the exact display_name and resource word. Do not add any other subject, cause, advice, heading, list, or metric number.
            """
        let options = GenerationOptions(sampling: .greedy, maximumResponseTokens: 64)
        let generated = try await session.respond(
            to: prompt,
            schema: try narrativeSchema(),
            options: options
        )
        let generatedSentence = try generated.content.value(String.self, forProperty: "sentence")
        guard let sentence = normalizeOneSentence(generatedSentence) else {
            return SidecarResponse(availability: .unsupported, result: nil)
        }
        return SidecarResponse(
            availability: .available,
            result: GenerationResult(
                publicationSequence: request.publicationSequence,
                factDigest: request.factDigest,
                text: sentence
            )
        )
    } catch is CancellationError {
        return SidecarResponse(availability: .busy, result: nil)
    } catch let error as LanguageModelSession.GenerationError {
        return SidecarResponse(availability: availability(for: error), result: nil)
    } catch {
        return SidecarResponse(availability: .runtimeMissing, result: nil)
    }
}

@available(macOS 26.0, *)
private func narrativeSchema() throws -> GenerationSchema {
    let sentence = DynamicGenerationSchema.Property(
        name: "sentence",
        description: "Exactly one plain sentence of at most 180 characters. State only qualitative facts from the supplied packet; do not state numbers, speculate, or give advice.",
        schema: DynamicGenerationSchema(type: String.self)
    )
    let root = DynamicGenerationSchema(
        name: "BatCaveNarrative",
        description: "A concise resource-monitor narrative grounded only in supplied facts.",
        properties: [sentence]
    )
    return try GenerationSchema(root: root, dependencies: [])
}

@available(macOS 26.0, *)
private func availability(
    for error: LanguageModelSession.GenerationError
) -> ProviderAvailability {
    switch error {
    case .assetsUnavailable:
        return .modelNotReady
    case .rateLimited, .concurrentRequests:
        return .busy
    case .exceededContextWindowSize,
         .guardrailViolation,
         .unsupportedGuide,
         .unsupportedLanguageOrLocale,
         .decodingFailure,
         .refusal:
        return .unsupported
    @unknown default:
        return .runtimeMissing
    }
}
#endif
