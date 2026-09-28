// Shows a ModelRenderer's output (premultiplied, in the model's sRGB-encoded
// colour) in a scene or UI. In a Linear colour-space project the colour is
// decoded so it blends like any other sprite. _Color tints and fades the
// whole model.
Shader "AetherCanvas/Display"
{
    Properties
    {
        [PerRendererData] _MainTex ("Model output", 2D) = "black" {}
        _Color ("Tint", Color) = (1, 1, 1, 1)
    }

    SubShader
    {
        Tags
        {
            "Queue" = "Transparent"
            "RenderType" = "Transparent"
            "IgnoreProjector" = "True"
            "PreviewType" = "Plane"
            "CanUseSpriteAtlas" = "False"
        }
        Cull Off
        ZWrite Off
        Lighting Off
        Blend One OneMinusSrcAlpha

        Pass
        {
            CGPROGRAM
            #pragma vertex vert
            #pragma fragment frag
            #include "UnityCG.cginc"

            sampler2D _MainTex;
            fixed4 _Color;

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

            float3 decodeSrgb(float3 c)
            {
                float3 low = c / 12.92;
                float3 high = pow((max(c, 0.0) + 0.055) / 1.055, 2.4);
                return lerp(high, low, step(c, 0.04045));
            }

            float4 frag(v2f i) : SV_Target
            {
                float4 c = tex2D(_MainTex, i.uv);
                float3 rgb = c.rgb;
            #ifndef UNITY_COLORSPACE_GAMMA
                rgb = decodeSrgb(rgb / max(c.a, 1e-5)) * c.a;
            #endif
                return float4(rgb * _Color.rgb, c.a) * _Color.a;
            }
            ENDCG
        }
    }
}
