using System.Text.Json;
using System.Text.Json.Serialization;

namespace BatCave.Core.Serialization;

public static class JsonDefaults
{
    public static JsonSerializerOptions SnakeCase { get; } = CreateSnakeCase();

    private static JsonSerializerOptions CreateSnakeCase()
    {
        JsonSerializerOptions options = new(JsonSerializerDefaults.General)
        {
            PropertyNamingPolicy = JsonNamingPolicy.SnakeCaseLower,
            DefaultIgnoreCondition = JsonIgnoreCondition.WhenWritingNull,
            WriteIndented = true,
        };
        options.Converters.Add(new JsonStringEnumConverter(JsonNamingPolicy.SnakeCaseLower));
        return options;
    }
}
