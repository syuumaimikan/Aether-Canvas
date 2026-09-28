// Draws Aether Canvas parts for ModelRenderer. Textures are premultiplied
// (pass 6 does that once per page); the blend states are the C API's:
//   normal   ONE, ONE_MINUS_SRC_ALPHA
//   multiply DST_COLOR, ONE_MINUS_SRC_ALPHA, then ONE_MINUS_DST_ALPHA, ONE
//   screen   ONE, ONE_MINUS_SRC_COLOR
//   add      ONE, ONE
// with alpha always ONE, ONE_MINUS_SRC_ALPHA (as the web player does).
Shader "Hidden/AetherCanvas/Part"
{
    Properties
    {
        _MainTex ("Texture", 2D) = "white" {}
    }

    CGINCLUDE
    #include "UnityCG.cginc"

    sampler2D _MainTex;
    Texture2D _AetherMask;
    float _AetherOpacity;
    float4 _AetherMultiply;
    float4 _AetherScreen;
    float _AetherUseMask;
    float _AetherDecode;

    struct appdata
    {
        float4 vertex : POSITION;
        float2 uv : TEXCOORD0;
    };

    struct v2f
    {
        float4 pos : SV_POSITION;
        float2 uv : TEXCOORD0;
    };

    v2f vert(appdata v)
    {
        v2f o;
        o.pos = UnityObjectToClipPos(v.vertex);
        o.uv = v.uv;
        return o;
    }

    // Tint on straight colour, applied to premultiplied colour, then opacity
    // and the clip mask (sampled at this pixel of the same-sized target).
    float4 fragPart(v2f i) : SV_Target
    {
        float4 c = tex2D(_MainTex, i.uv);
        float3 rgb = c.rgb * _AetherMultiply.rgb;
        rgb = rgb + _AetherScreen.rgb * c.a - rgb * _AetherScreen.rgb;
        float k = _AetherOpacity;
        if (_AetherUseMask > 0.5)
            k *= _AetherMask.Load(int3(i.pos.xy, 0)).r;
        return float4(rgb, c.a) * k;
    }

    // The mask part's coverage times its opacity.
    float4 fragMask(v2f i) : SV_Target
    {
        float a = tex2D(_MainTex, i.uv).a * _AetherOpacity;
        return float4(a, a, a, a);
    }

    // sRGB encoding, exact, so decoded 8-bit values round-trip.
    float3 encodeSrgb(float3 c)
    {
        float3 low = c * 12.92;
        float3 high = 1.055 * pow(max(c, 0.0), 1.0 / 2.4) - 0.055;
        return lerp(high, low, step(c, 0.0031308));
    }

    // Premultiply a texture page (Graphics.Blit), undoing the sampler's sRGB
    // decode in Linear colour-space projects.
    float4 fragPremultiply(v2f i) : SV_Target
    {
        float4 c = tex2D(_MainTex, i.uv);
        if (_AetherDecode > 0.5)
            c.rgb = encodeSrgb(c.rgb);
        return float4(c.rgb * c.a, c.a);
    }
    ENDCG

    SubShader
    {
        Tags { "RenderType" = "Transparent" "Queue" = "Transparent" }
        Cull Off
        ZWrite Off
        ZTest Always

        Pass // 0: normal
        {
            Name "Normal"
            Blend One OneMinusSrcAlpha, One OneMinusSrcAlpha
            CGPROGRAM
            #pragma target 3.5
            #pragma vertex vert
            #pragma fragment fragPart
            ENDCG
        }

        Pass // 1: multiply, colour: Cs·Cd + Cd·(1−αs), alpha untouched
        {
            Name "MultiplyColour"
            Blend DstColor OneMinusSrcAlpha, Zero One
            CGPROGRAM
            #pragma target 3.5
            #pragma vertex vert
            #pragma fragment fragPart
            ENDCG
        }

        Pass // 2: multiply, then + Cs·(1−αd), and alpha source-over
        {
            Name "MultiplyAlpha"
            Blend OneMinusDstAlpha One, One OneMinusSrcAlpha
            CGPROGRAM
            #pragma target 3.5
            #pragma vertex vert
            #pragma fragment fragPart
            ENDCG
        }

        Pass // 3: screen
        {
            Name "Screen"
            Blend One OneMinusSrcColor, One OneMinusSrcAlpha
            CGPROGRAM
            #pragma target 3.5
            #pragma vertex vert
            #pragma fragment fragPart
            ENDCG
        }

        Pass // 4: add
        {
            Name "Add"
            Blend One One, One OneMinusSrcAlpha
            CGPROGRAM
            #pragma target 3.5
            #pragma vertex vert
            #pragma fragment fragPart
            ENDCG
        }

        Pass // 5: clip mask
        {
            Name "Mask"
            Blend One OneMinusSrcAlpha
            CGPROGRAM
            #pragma target 3.5
            #pragma vertex vert
            #pragma fragment fragMask
            ENDCG
        }

        Pass // 6: premultiply a texture page
        {
            Name "Premultiply"
            Blend Off
            CGPROGRAM
            #pragma target 3.5
            #pragma vertex vert
            #pragma fragment fragPremultiply
            ENDCG
        }
    }
}
