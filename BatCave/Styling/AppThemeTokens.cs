using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Media;
using System.Runtime.InteropServices;
using Windows.UI;

namespace BatCave.Styling;

internal static class AppThemeTokens
{
    public static Color ResolveColor(string key, Color fallback)
    {
        if (TryGetResource(key, out object? resource)
            && resource is Color color)
        {
            return color;
        }

        return fallback;
    }

    public static Brush ResolveBrush(string key, Color fallbackColor)
    {
        if (TryGetResource(key, out object? resource)
            && resource is Brush brush)
        {
            return brush;
        }

        return new SolidColorBrush(fallbackColor);
    }

    private static bool TryGetResource(string key, out object? resource)
    {
        try
        {
            resource = null;
            Application? application = Application.Current;
            return application?.Resources.TryGetValue(key, out resource) == true;
        }
        catch (COMException)
        {
            // Unit tests can execute without WinUI initialization.
            resource = null;
            return false;
        }
    }
}
