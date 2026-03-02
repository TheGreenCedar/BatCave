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

        Array.Sort(scratch, 0, scratch.Length);
        int percentileIndex = Math.Min(scratch.Length - 1, Math.Max(0, (int)Math.Ceiling(scratch.Length * 0.95) - 1));
        return scratch[percentileIndex];
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

        Array.Sort(scratch, 0, copyCount);
        int percentileIndex = Math.Min(copyCount - 1, Math.Max(0, (int)Math.Ceiling(copyCount * 0.95) - 1));
        return scratch[percentileIndex];
    }
}
