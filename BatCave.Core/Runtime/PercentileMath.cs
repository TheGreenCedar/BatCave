namespace BatCave.Core.Runtime;

internal static class PercentileMath
{
    public static double Percentile95(IReadOnlyList<double> values)
    {
        if (values.Count == 0)
        {
            return 0;
        }

        double[] scratch = new double[values.Count];
        for (int index = 0; index < values.Count; index++)
        {
            scratch[index] = values[index];
        }

        return Percentile95FromBuffer(scratch, scratch.Length);
    }

    public static double Percentile95(double[] values, int count, double[] scratch)
    {
        if (count <= 0)
        {
            return 0;
        }

        int copyCount = Math.Min(Math.Min(count, values.Length), scratch.Length);
        if (copyCount <= 0)
        {
            return 0;
        }

        for (int index = 0; index < copyCount; index++)
        {
            scratch[index] = values[index];
        }

        return Percentile95FromBuffer(scratch, copyCount);
    }

    private static double Percentile95FromBuffer(double[] buffer, int count)
    {
        Array.Sort(buffer, 0, count);
        int percentileIndex = ResolvePercentile95Index(count);
        return buffer[percentileIndex];
    }

    private static int ResolvePercentile95Index(int count)
    {
        return Math.Min(count - 1, Math.Max(0, (int)Math.Ceiling(count * 0.95) - 1));
    }
}
