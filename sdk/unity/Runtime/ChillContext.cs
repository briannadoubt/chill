using System;
using UnityEngine;

namespace Chill.Unity
{
    [Serializable]
    public sealed class ChillStringAnnotation
    {
        [SerializeField]
        private string key = "";

        [SerializeField]
        private string value = "";

        [SerializeField]
        private ChillAnnotationClassification classification = ChillAnnotationClassification.Internal;

        public string Key
        {
            get { return key; }
        }

        public string Value
        {
            get { return value; }
        }

        public ChillAnnotationClassification Classification
        {
            get { return classification; }
        }
    }

    [DisallowMultipleComponent]
    [AddComponentMenu("Chill/Semantic Context")]
    public sealed class ChillContext : MonoBehaviour
    {
        [SerializeField]
        private ChillStringAnnotation[] annotations = new ChillStringAnnotation[0];

        internal ChillAnnotations DeclaredAnnotations()
        {
            var result = new ChillAnnotations();
            for (int index = 0; index < annotations.Length; index += 1)
            {
                ChillStringAnnotation annotation = annotations[index];
                if (annotation == null || !ChillNames.IsAnnotationKey(annotation.Key))
                {
                    continue;
                }
                ChillRuntime.DeclareAnnotation(annotation.Key, annotation.Classification);
                try
                {
                    result = result.With(annotation.Key, annotation.Value);
                }
                catch (ArgumentException)
                {
                    // Invalid bounded values are omitted at their declaration source.
                }
            }
            return result;
        }

        internal static ChillAnnotations Resolve(Transform leaf)
        {
            ChillContext[] contexts = leaf.GetComponentsInParent<ChillContext>(true);
            var resolved = new ChillAnnotations();
            for (int index = contexts.Length - 1; index >= 0; index -= 1)
            {
                resolved = resolved.MergeDescendant(contexts[index].DeclaredAnnotations());
            }
            return resolved;
        }
    }
}
