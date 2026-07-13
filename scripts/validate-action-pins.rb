# frozen_string_literal: true

require "json"
require "pathname"
require "psych"

IMMUTABLE_COMMIT = /\A[0-9a-f]{40}\z/.freeze
IMMUTABLE_CONTAINER = /\Adocker:\/\/[^\s@]+@sha256:[0-9a-f]{64}\z/.freeze
REMOTE_ACTION = /\A[^\/@\s]+\/[^\/@\s]+(?:\/[^@\s]+)*\z/.freeze

def workflow_files(repo_root)
  Dir.glob(File.join(repo_root, ".github", "workflows", "**", "*.{yml,yaml}"))
    .select { |file| File.file?(file) }
    .sort
end

def inline_comment(lines, node)
  return nil unless node.start_line == node.end_line

  suffix = lines.fetch(node.end_line, "")[node.end_column..-1].to_s
  single_quoted = false
  double_quoted = false
  escaped = false
  suffix.each_char.with_index do |character, index|
    if double_quoted
      if escaped
        escaped = false
      elsif character == "\\"
        escaped = true
      elsif character == '"'
        double_quoted = false
      end
    elsif single_quoted
      single_quoted = false if character == "'"
    elsif character == '"'
      double_quoted = true
    elsif character == "'"
      single_quoted = true
    elsif character == "#" && (index.zero? || suffix[index - 1] =~ /\s/)
      return suffix[(index + 1)..-1].to_s.strip
    end
  end
  nil
end

def pin_violation(value, comment)
  return nil if value.start_with?("./")

  if value.start_with?("docker://")
    unless IMMUTABLE_CONTAINER.match?(value)
      return "external Docker action must use an exact lowercase sha256 image digest"
    end
    return "immutable Docker action pin must retain a readable version comment" if comment.to_s.empty?

    return nil
  end

  separator = value.rindex("@")
  return "external action is missing an immutable commit reference" unless separator&.positive?

  action = value[0...separator]
  ref = value[(separator + 1)..-1]
  return "external action must use owner/repository[/path] syntax" unless REMOTE_ACTION.match?(action)

  unless IMMUTABLE_COMMIT.match?(ref)
    return "external action ref must be an exact lowercase 40-character commit SHA; received #{ref}"
  end
  return "immutable action pin must retain its readable version as an inline comment" if comment.to_s.empty?

  nil
end

def collect_anchors(node, anchors)
  if node.respond_to?(:anchor) && node.anchor && !node.is_a?(Psych::Nodes::Alias)
    anchors[node.anchor] = node
  end
  return unless node.respond_to?(:children) && node.children

  node.children.each { |child| collect_anchors(child, anchors) }
end

def scalar_node(node, anchors)
  return node if node.is_a?(Psych::Nodes::Scalar)
  return anchors[node.anchor] if node.is_a?(Psych::Nodes::Alias) && anchors[node.anchor].is_a?(Psych::Nodes::Scalar)

  nil
end

def collect_uses_nodes(node, lines, file, violations, anchors)
  if node.is_a?(Psych::Nodes::Mapping)
    node.children.each_slice(2) do |key, value|
      resolved_key = scalar_node(key, anchors)
      if resolved_key&.value == "uses"
        resolved_value = scalar_node(value, anchors)
        if resolved_value
          message = pin_violation(resolved_value.value, inline_comment(lines, value))
          violations << { file: file, line: value.start_line + 1, message: message } if message
        else
          violations << {
            file: file,
            line: value.start_line + 1,
            message: "uses entries must be scalar values"
          }
        end
      end
      collect_uses_nodes(value, lines, file, violations, anchors)
    end
  elsif node.respond_to?(:children) && node.children
    node.children.each { |child| collect_uses_nodes(child, lines, file, violations, anchors) }
  end
end

def collect_violations(repo_root)
  violations = []
  workflow_files(repo_root).each do |absolute_file|
    file = Pathname.new(absolute_file).relative_path_from(Pathname.new(repo_root)).to_s
    source = File.read(absolute_file)
    lines = source.lines(chomp: true)
    begin
      tree = Psych.parse_stream(source, filename: absolute_file)
      anchors = {}
      collect_anchors(tree, anchors)
      collect_uses_nodes(tree, lines, file, violations, anchors)
    rescue Psych::SyntaxError => error
      violations << {
        file: file,
        line: error.line,
        message: "workflow YAML could not be parsed: #{error.problem}"
      }
    end
  end
  violations.sort_by { |violation| [violation[:file], violation[:line], violation[:message]] }
end

json = ARGV.delete("--json")
repo_root = File.expand_path(ARGV.shift || Dir.pwd)
violations = collect_violations(repo_root)
if json
  puts JSON.generate(violations)
elsif violations.empty?
  puts "GitHub Actions references are pinned to immutable commits or digests"
else
  violations.each do |violation|
    warn "#{violation[:file]}:#{violation[:line]}: #{violation[:message]}"
  end
  exit 1
end
