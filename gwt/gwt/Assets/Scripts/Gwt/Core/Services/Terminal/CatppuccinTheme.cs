using UnityEngine;

namespace Gwt.Core.Services.Terminal
{
    public static class CatppuccinTheme
    {
        // Catppuccin Mocha 16-color palette
        public static readonly Color[] Colors = new Color[16]
        {
            new(0.18f, 0.19f, 0.25f, 1f),  // 0  Black (Surface0)
            new(0.95f, 0.55f, 0.66f, 1f),  // 1  Red
            new(0.65f, 0.89f, 0.63f, 1f),  // 2  Green
            new(0.98f, 0.90f, 0.60f, 1f),  // 3  Yellow
            new(0.54f, 0.71f, 0.98f, 1f),  // 4  Blue
            new(0.80f, 0.62f, 0.95f, 1f),  // 5  Magenta
            new(0.58f, 0.89f, 0.87f, 1f),  // 6  Cyan
            new(0.80f, 0.83f, 0.90f, 1f),  // 7  White (Subtext1)
            new(0.27f, 0.28f, 0.35f, 1f),  // 8  Bright Black (Surface1)
            new(0.95f, 0.55f, 0.66f, 1f),  // 9  Bright Red
            new(0.65f, 0.89f, 0.63f, 1f),  // 10 Bright Green
            new(0.98f, 0.90f, 0.60f, 1f),  // 11 Bright Yellow
            new(0.54f, 0.71f, 0.98f, 1f),  // 12 Bright Blue
            new(0.80f, 0.62f, 0.95f, 1f),  // 13 Bright Magenta
            new(0.58f, 0.89f, 0.87f, 1f),  // 14 Bright Cyan
            new(0.81f, 0.85f, 0.92f, 1f),  // 15 Bright White (Text)
        };

        public static Color Background => new(0.12f, 0.12f, 0.18f, 1f);  // Base
        public static Color Foreground => Colors[15];
        public static Color CursorColor => new(0.95f, 0.76f, 0.66f, 1f); // Rosewater
    }
}
